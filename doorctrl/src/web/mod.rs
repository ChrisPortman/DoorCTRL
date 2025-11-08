use core::{ops::DerefMut, str};

use conf::{ConfigV1, ConfigV1Update};
use defmt::{error, info, warn};
use embassy_futures::select;
use embassy_net::{tcp::TcpSocket, IpListenEndpoint, Stack};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Sender, mutex::Mutex, pubsub::Subscriber,
};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use esp_bootloader_esp_idf::partitions::FlashRegion;
use esp_hal::system::software_reset;
use esp_storage::FlashStorage;

use crate::state::{AnyState, DoorState, LockState};

use http::{
    self,
    header::HttpHeader,
    request::{HttpMethod, HttpRequest},
    response::{HttpResponse, HttpStatusCode},
    websocket::{sec_websocket_accept_val, WebsocketError, WebsocketFrame},
    HTTPError,
};

const MAX_REQUEST_HDRS: usize = 16;
const MAX_RESPONSE_HDRS: usize = 6;

const ERR_ACCEPT_ABORTED: &'static str = "waiting for connection aborted";

const WS_STATE_UPDATE: u8 = 1;
const WS_CONFIG_UPDATE: u8 = 2;
const WS_NOTIFICATION: u8 = 3;

// state update payloads
const WS_LOCK_LOCK: u8 = 1;
const WS_LOCK_UNLOCK: u8 = 2;
const WS_DOOR_OPEN: u8 = 3;
const WS_DOOR_CLOSED: u8 = 4;

const HTML_INDEX: &'static [u8] = include_bytes!("html/index.html");
const HTML_404: &'static [u8] = include_bytes!("html/404.html");
const HTML_400: &'static [u8] = include_bytes!("html/400.html");
const FAVICON: &'static [u8] = include_bytes!("html/favicon.ico");

type Storage = &'static Mutex<CriticalSectionRawMutex, FlashRegion<'static, FlashStorage<'static>>>;

pub struct HttpService {
    storage: Storage,
    config: ConfigV1,
    door_state: Option<DoorState>,
    lock_state: Option<LockState>,
}

impl HttpService {
    pub fn new(config: ConfigV1, storage: Storage) -> Self {
        Self {
            storage: storage,
            config: config,
            door_state: None,
            lock_state: None,
        }
    }

    fn handle_request(
        &self,
        req: &HttpRequest<MAX_REQUEST_HDRS>,
        resp: &mut HttpResponse<MAX_RESPONSE_HDRS>,
    ) -> Result<Option<&'static [u8]>, HTTPError> {
        if req.content_len() > 0 {
            // We dont take requests with payloads, so rather than have
            // to handle syncronising the tcp stream by reading off the
            // content we dont want, we'll just disconnect the client
            // and start fresh.
            resp.set_status(HttpStatusCode::BadRequest);
            return Err(HTTPError::UnsupportedRequest(
                "requests with bodies is not supported",
            ));
        }

        match req.method {
            HttpMethod::GET => {}
            _ => {
                error!("only GET request are supported");
                resp.set_status(HttpStatusCode::NotFound);
                return Ok(Some(HTML_404));
            }
        }

        match req.path {
            "/" => return Ok(Some(HTML_INDEX)),
            "/favicon.ico" => Ok(Some(FAVICON)),
            "/ws" => {
                if let Some(HttpHeader::SecWebSocketKey(key)) =
                    req.get_header(HttpHeader::SecWebSocketKey(""))
                {
                    if let Ok(accept_key) = sec_websocket_accept_val(key) {
                        resp.set_status(HttpStatusCode::SwitchingProtocols);
                        if let Err(e) = resp
                            .add_extra_header(HttpHeader::SecWebSocketAccept(accept_key))
                            .and(resp.add_extra_header(HttpHeader::Other("Upgrade", "websocket")))
                            .and(resp.add_extra_header(HttpHeader::Other("Connection", "Upgrade")))
                        {
                            return Err(e);
                        }

                        return Ok(None);
                    }
                }
                resp.set_status(HttpStatusCode::BadRequest);
                return Ok(Some(HTML_400));
            }
            _ => {
                resp.set_status(HttpStatusCode::NotFound);
                return Ok(Some(HTML_404));
            }
        }
    }

    pub async fn receive_request<'a, 'b>(
        &self,
        sock: &mut TcpSocket<'a>,
        buff: &'b mut [u8],
    ) -> Result<HttpRequest<'b, MAX_REQUEST_HDRS>, HTTPError> {
        let mut offset = 0usize;
        let req_len: usize;
        let req: HttpRequest<'b, MAX_REQUEST_HDRS>;

        loop {
            let read = match sock.read(&mut buff[offset..]).await {
                Ok(0) => return Err(HTTPError::NetworkError("client closed connection")),
                Ok(n) => n,
                Err(_) => return Err(HTTPError::NetworkError("Error reading from connection")),
            };

            offset += read;

            if let Some(pos) = HttpRequest::<MAX_REQUEST_HDRS>::contains_request_headers(&*buff) {
                req_len = pos;
                break;
            }
        }

        req = HttpRequest::parse_request(&buff[..req_len])?;
        return Ok(req);
    }

    pub async fn run<'a, 'b>(
        &mut self,
        stack: Stack<'static>,
        cmd_channel: &Sender<'static, CriticalSectionRawMutex, LockState, 2>,
        state_sub: &mut Subscriber<'static, CriticalSectionRawMutex, AnyState, 2, 6, 0>,
    ) -> Result<(), &'static str> {
        let endpoint = IpListenEndpoint {
            addr: None,
            port: 80,
        };

        let mut tcp_rx_buff = [0u8; 1024];
        let mut tcp_tx_buff = [0u8; 1024];
        let mut http_buf = [0u8; 1024];

        loop {
            // each iteration handles an HTTP client
            info!("waiting for http connection");
            let mut sock = TcpSocket::new(stack, &mut tcp_rx_buff, &mut tcp_tx_buff);
            if let Err(e) = sock.accept(endpoint).await {
                error!("http: error waiting for connection: {}", e);
                return Err(ERR_ACCEPT_ABORTED);
            }
            info!("connection from {}", sock.remote_endpoint());

            'request: loop {
                // each iteration handles an HTTP request/response
                let req = match select::select(
                    self.receive_request(&mut sock, &mut http_buf),
                    Timer::after(Duration::from_secs(1)),
                )
                .await
                {
                    select::Either::First(Ok(r)) => r,
                    select::Either::First(Err(e)) => {
                        error!("web: error receiving request: {:?}", e);
                        break 'request;
                    }
                    select::Either::Second(_) => {
                        info!("web: closing idle client connection");
                        break 'request;
                    }
                };

                info!(
                    "http: request to {} {} with {} bytes payload: {:?}",
                    req.method,
                    req.path,
                    req.content_len(),
                    req,
                );

                let mut resp = HttpResponse::default();
                let mut body: Option<&'static [u8]> = None;
                let mut upgrade: bool = false;

                match self.handle_request(&req, &mut resp) {
                    Ok(Some(b)) => body = Some(b),
                    Ok(None) => {}
                    Err(e) => {
                        error!("web: processing request experianced error: {:?}", e);
                        break 'request;
                    }
                };

                if let HttpStatusCode::SwitchingProtocols = resp.get_status() {
                    upgrade = true;
                }

                if let Err(e) = match body {
                    Some(b) => resp.send_with_body(&mut sock, b).await,
                    None => resp.send(&mut sock).await,
                } {
                    error!("web: error sending response - {:?}", e);
                }

                if upgrade {
                    self.run_ws(&mut sock, &mut http_buf, cmd_channel, state_sub)
                        .await?;
                    break 'request;
                }
            }

            sock.close();
        }
    }

    async fn send_config_via_ws<T: Write>(&self, mut writer: &mut T) {
        let mut serialized = [0u8; 1024];
        serialized[0] = WS_CONFIG_UPDATE;

        match serde_json_core::to_slice(&self.config, &mut serialized[1..]) {
            Ok(mut n) => {
                n += 1; // account for the leading message type indicator
                if let Err(e) = WebsocketFrame::send(&mut writer, &mut serialized[..n]).await {
                    error!("error sending config to web client: {}", e);
                }
            }
            Err(e) => error!("error serializing config to send to web client: {}", e),
        }
    }

    async fn send_state_via_ws<T: Write>(
        &mut self,
        mut writer: &mut T,
        state: AnyState,
    ) -> Result<(), &'static str> {
        if let Err(e) = match state {
            AnyState::LockState(LockState::Locked) => {
                self.lock_state = Some(LockState::Locked);
                WebsocketFrame::send(&mut writer, &mut [WS_STATE_UPDATE, WS_LOCK_LOCK]).await
            }
            AnyState::LockState(LockState::Unlocked) => {
                self.lock_state = Some(LockState::Unlocked);
                WebsocketFrame::send(&mut writer, &mut [WS_STATE_UPDATE, WS_LOCK_UNLOCK]).await
            }
            AnyState::DoorState(DoorState::Open) => {
                self.door_state = Some(DoorState::Open);
                WebsocketFrame::send(&mut writer, &mut [WS_STATE_UPDATE, WS_DOOR_OPEN]).await
            }
            AnyState::DoorState(DoorState::Closed) => {
                self.door_state = Some(DoorState::Closed);
                WebsocketFrame::send(&mut writer, &mut [WS_STATE_UPDATE, WS_DOOR_CLOSED]).await
            }
        } {
            error!("websocket: error writing to socket: {}", e);
            return Err("error writing to websocket");
        }

        Ok(())
    }

    async fn send_notification_via_ws<T: Write>(
        &mut self,
        mut writer: &mut T,
        notif: &[u8],
    ) -> Result<(), &'static str> {
        if let Err(e) =
            WebsocketFrame::send(&mut writer, &mut [&[WS_NOTIFICATION], notif].concat()).await
        {
            error!("websocket: error writing to socket: {}", e);
            return Err("error writing to websocket");
        }

        info!("notification sent to client");

        Ok(())
    }

    pub async fn run_ws<'a, 'b>(
        &mut self,
        sock: &mut TcpSocket<'b>,
        mut buff: &mut [u8],
        cmd_channel: &Sender<'static, CriticalSectionRawMutex, LockState, 2>,
        state_sub: &mut Subscriber<'static, CriticalSectionRawMutex, AnyState, 2, 6, 0>,
    ) -> Result<(), &'static str> {
        let (mut reader, mut writer) = sock.split();

        // For the first client on the task, there will be states in the state sub queue.
        // For subsequent clients, we will need to send retined states.
        if let Some(door_state) = self.door_state {
            self.send_state_via_ws(&mut writer, AnyState::DoorState(door_state))
                .await?;
        }
        if let Some(lock_state) = self.lock_state {
            self.send_state_via_ws(&mut writer, AnyState::LockState(lock_state))
                .await?;
        }

        self.send_config_via_ws(&mut writer).await;

        loop {
            info!("websocket: waiting for state update or data from client");
            buff.fill(0u8);
            match select::select(
                WebsocketFrame::receive(&mut reader, &mut buff),
                state_sub.next_message_pure(),
            )
            .await
            {
                select::Either::First(Ok(ws)) => {
                    info!("websocket: processing client data");

                    if ws.opcode == 8 {
                        // connection close
                        return Ok(());
                    }

                    let data = &buff[..ws.len];
                    if data.len() < 2 {
                        error!("websocket messages should have at least 2 bytes of data");
                        return Err("websocket protocol err");
                    }

                    match data[0] {
                        WS_STATE_UPDATE => match data[1] {
                            WS_LOCK_LOCK => cmd_channel.send(LockState::Locked).await,
                            WS_LOCK_UNLOCK => cmd_channel.send(LockState::Unlocked).await,
                            _ => warn!("received unknown state update from websocket: {}", buff[0]),
                        },
                        WS_CONFIG_UPDATE => {
                            info!("{}", str::from_utf8(&data[1..]).unwrap_or("not urf8"));
                            match serde_json_core::from_slice::<ConfigV1Update>(&data[1..]) {
                                Ok((update, _)) => {
                                    self.config.update(&update);
                                    info!("config updated");
                                    info!("device name: {}", self.config.device_name.as_str());
                                    info!("wifi_ssid: {}", self.config.wifi_ssid.as_str());
                                    info!("wifi_pass: {}", self.config.wifi_pass.as_str());
                                    info!("mqtt_host: {}", self.config.mqtt_host.as_str());
                                    info!("mqtt_user: {}", self.config.mqtt_user.as_str());
                                    info!("mqtt_pass: {}", self.config.mqtt_pass.as_str());

                                    let mut locked_storage = self.storage.lock().await;
                                    match self.config.save(locked_storage.deref_mut()) {
                                        Ok(()) => {
                                            info!("config saved. rebooting");
                                            self.send_notification_via_ws(
                                                &mut writer,
                                                "Config saved, rebooting...".as_bytes(),
                                            )
                                            .await?;

                                            Timer::after(Duration::from_secs(1)).await;
                                            software_reset();
                                        }
                                        Err(e) => {
                                            error!("failed to save config: {}", e);
                                            self.send_notification_via_ws(
                                                &mut writer,
                                                e.as_bytes(),
                                            )
                                            .await?;
                                        }
                                    }
                                    drop(locked_storage);
                                }
                                Err(e) => {
                                    error!("received invalid data: {}", e);
                                }
                            }
                        }
                        _ => {
                            error!("websocket: received unknown payload type: {}", buff[0]);
                            return Err("received unknown payload type");
                        }
                    }
                }
                select::Either::First(Err(e @ WebsocketError::NetworkError)) => {
                    info!("websocket: {:?}", e);
                    return Ok(());
                }
                select::Either::First(Err(e)) => {
                    error!("websocket: error receiving websocket frame: {:?}", e);
                    return Err("websocket finished with error");
                }
                select::Either::Second(state) => {
                    info!("websocket: processing state update");
                    self.send_state_via_ws(&mut writer, state).await?;
                }
            }
        }
    }
}
