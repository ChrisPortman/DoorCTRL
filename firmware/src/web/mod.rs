use core::{ops::DerefMut, str};

use defmt::{error, info, warn};
use embassy_futures::select;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Sender, mutex::Mutex,
    pubsub::PubSubChannel,
};
use embassy_time::{Duration, Timer};
use embedded_io_async::{Read, Write};
use esp_bootloader_esp_idf::partitions::FlashRegion;
use esp_hal::system::software_reset;
use esp_storage::FlashStorage;

use doorctrl::config::{ConfigV1, ConfigV1Update};
use doorctrl::state::{AnyState, DoorState, LockState};
use weblite::{
    request::Request,
    response::{Responder, StatusCode},
    server::HandlerError,
    server::RequestHandler,
    websocket::{Websocket, WebsocketError},
};

const WS_STATE_UPDATE: u8 = 1;
const WS_CONFIG_UPDATE: u8 = 2;
const WS_NOTIFICATION: u8 = 3;

// state update payloads
const WS_LOCK_LOCK: u8 = 1;
const WS_LOCK_UNLOCK: u8 = 2;
const WS_DOOR_OPEN: u8 = 3;
const WS_DOOR_CLOSED: u8 = 4;

const HTML_INDEX: &[u8] = include_bytes!("html/index.html");
const HTML_404: &[u8] = include_bytes!("html/404.html");
const FAVICON: &[u8] = include_bytes!("html/favicon.ico");

type Storage = &'static Mutex<CriticalSectionRawMutex, FlashRegion<'static, FlashStorage<'static>>>;

pub struct HttpServiceState {
    pub storage: Storage,
    pub config: ConfigV1,
    pub door_state: Option<DoorState>,
    pub lock_state: Option<LockState>,
}

pub struct HttpClientHandler {
    inner: Mutex<CriticalSectionRawMutex, HttpServiceState>,
    cmd_channel: Sender<'static, CriticalSectionRawMutex, LockState, 2>,
    state_updates: &'static PubSubChannel<CriticalSectionRawMutex, AnyState, 2, 6, 0>,
}

impl RequestHandler for HttpClientHandler {
    async fn handle_request<'client, 'buff, C: Read + Write + 'client>(
        &self,
        req: Request<'buff>,
        resp: Responder<'buff, 'client, C>,
    ) -> Result<Option<Websocket<'client, C>>, HandlerError> {
        match req.path {
            "/" => {
                resp.with_status(StatusCode::OK)
                    .await?
                    .with_body(HTML_INDEX)
                    .await?;
            }
            "/favicon.ico" => {
                resp.with_status(StatusCode::OK)
                    .await?
                    .with_body(FAVICON)
                    .await?;
            }
            "/ws" => {
                return Ok(Some(resp.upgrade(req).await?));
            }
            _ => {
                resp.with_status(StatusCode::NotFound)
                    .await?
                    .with_body(HTML_404)
                    .await?;
            }
        }

        Ok(None)
    }

    async fn handle_websocket<'client, C: Read + Write + 'client>(
        &self,
        mut websocket: Websocket<'client, C>,
        buffer: &mut [u8],
    ) -> Result<(), HandlerError> {
        if let Err(e) = self.run_ws(&mut websocket, buffer).await {
            error!("run_ws returned error: {}", e);
            return Err(e);
        }
        Ok(())
    }
}

impl HttpClientHandler {
    pub fn new(
        inner: HttpServiceState,
        cmd_channel: Sender<'static, CriticalSectionRawMutex, LockState, 2>,
        state_updates: &'static PubSubChannel<CriticalSectionRawMutex, AnyState, 2, 6, 0>,
    ) -> Self {
        Self {
            inner: Mutex::new(inner),
            cmd_channel,
            state_updates,
        }
    }

    async fn send_config_via_ws<'a, C>(
        &self,
        socket: &mut Websocket<'a, C>,
    ) -> Result<(), HandlerError>
    where
        C: Read + Write,
    {
        let mut serialized = [0u8; 1024];
        serialized[0] = WS_CONFIG_UPDATE;

        let inner = self.inner.lock().await;
        match serde_json_core::to_slice(&inner.config, &mut serialized[1..]) {
            Ok(mut n) => {
                n += 1; // account for the leading message type indicator
                if let Err(e) = socket.send(&mut serialized[..n]).await {
                    error!("error sending config to web client: {}", e);
                    return Err(HandlerError::WebsocketError(e));
                }
            }
            Err(e) => {
                error!("error serializing config to send to web client: {}", e);
                return Err(HandlerError::CustomError("serializing config failed"));
            }
        }

        Ok(())
    }

    async fn send_state_via_ws<'a, C>(
        &self,
        socket: &mut Websocket<'a, C>,
        state: AnyState,
    ) -> Result<(), WebsocketError>
    where
        C: Read + Write,
    {
        if let Err(e) = match state {
            AnyState::LockState(LockState::Locked) => {
                socket.send(&mut [WS_STATE_UPDATE, WS_LOCK_LOCK]).await
            }
            AnyState::LockState(LockState::Unlocked) => {
                socket.send(&mut [WS_STATE_UPDATE, WS_LOCK_UNLOCK]).await
            }
            AnyState::DoorState(DoorState::Open) => {
                socket.send(&mut [WS_STATE_UPDATE, WS_DOOR_OPEN]).await
            }
            AnyState::DoorState(DoorState::Closed) => {
                socket.send(&mut [WS_STATE_UPDATE, WS_DOOR_CLOSED]).await
            }
        } {
            error!("websocket: error writing to socket: {}", e);
            return Err(e);
        };

        Ok(())
    }

    async fn send_notification_via_ws<'a, C>(
        &self,
        socket: &mut Websocket<'a, C>,
        notif: &[u8],
    ) -> Result<(), WebsocketError>
    where
        C: Read + Write,
    {
        if let Err(e) = socket.send(&mut [&[WS_NOTIFICATION], notif].concat()).await {
            error!("websocket: error writing to socket: {}", e);
            return Err(e);
        }

        info!("notification sent to client");

        Ok(())
    }

    async fn run_ws<'a, C>(
        &self,
        socket: &mut Websocket<'a, C>,
        buffer: &mut [u8],
    ) -> Result<(), HandlerError>
    where
        C: Read + Write,
    {
        // For the first client on the task, there will be states in the state sub queue.
        // For subsequent clients, we will need to send retined states.
        {
            let inner = self.inner.lock().await;
            if let Some(door_state) = inner.door_state {
                self.send_state_via_ws(socket, AnyState::DoorState(door_state))
                    .await?;
            }
            if let Some(lock_state) = inner.lock_state {
                self.send_state_via_ws(socket, AnyState::LockState(lock_state))
                    .await?;
            }
        }

        self.send_config_via_ws(socket).await?;

        let mut state_sub = match self.state_updates.subscriber() {
            Ok(s) => s,
            Err(_) => {
                return Err(HandlerError::CustomError(
                    "webseocket process upable to subscribe to state updates",
                ));
            }
        };

        loop {
            info!("websocket: waiting for state update or data from client");
            match select::select(socket.receive(buffer), state_sub.next_message_pure()).await {
                select::Either::First(Ok(ws)) => {
                    info!("websocket: processing client data");

                    if ws.opcode == 8 {
                        // connection close
                        return Ok(());
                    }

                    let data = &buffer[..ws.len];
                    if data.len() < 2 {
                        error!("websocket messages should have at least 2 bytes of data");
                        return Err(HandlerError::WebsocketError(
                            WebsocketError::InsufficientData(2),
                        ));
                    }

                    match data[0] {
                        WS_STATE_UPDATE => match data[1] {
                            WS_LOCK_LOCK => self.cmd_channel.send(LockState::Locked).await,
                            WS_LOCK_UNLOCK => self.cmd_channel.send(LockState::Unlocked).await,
                            _ => warn!(
                                "received unknown state update from websocket: {}",
                                buffer[0]
                            ),
                        },
                        WS_CONFIG_UPDATE => {
                            info!("{}", str::from_utf8(&data[1..]).unwrap_or("not urf8"));
                            match serde_json_core::from_slice::<ConfigV1Update>(&data[1..]) {
                                Ok((update, _)) => {
                                    let mut inner = self.inner.lock().await;
                                    inner.config.update(&update);
                                    info!("config updated");
                                    info!("device name: {}", inner.config.device_name.as_str());
                                    info!("wifi_ssid: {}", inner.config.wifi_ssid.as_str());
                                    info!("wifi_pass: {}", inner.config.wifi_pass.as_str());
                                    info!("mqtt_host: {}", inner.config.mqtt_host.as_str());
                                    info!("mqtt_user: {}", inner.config.mqtt_user.as_str());
                                    info!("mqtt_pass: {}", inner.config.mqtt_pass.as_str());

                                    let mut locked_storage = inner.storage.lock().await;
                                    match inner.config.save(locked_storage.deref_mut()) {
                                        Ok(()) => {
                                            info!("config saved. rebooting");
                                            self.send_notification_via_ws(
                                                socket,
                                                "Config saved, rebooting...".as_bytes(),
                                            )
                                            .await?;

                                            Timer::after(Duration::from_secs(1)).await;
                                            software_reset();
                                        }
                                        Err(e) => {
                                            error!("failed to save config: {}", e);
                                            self.send_notification_via_ws(socket, e.as_bytes())
                                                .await?;
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("received invalid data: {}", e);
                                }
                            }
                        }
                        _ => {
                            error!("websocket: received unknown payload type: {}", buffer[0]);
                            return Err(HandlerError::CustomError("received unknown payload type"));
                        }
                    }
                }
                select::Either::First(Err(e)) => {
                    error!("websocket: error receiving websocket frame: {:?}", e);
                    return Err(HandlerError::WebsocketError(e));
                }
                select::Either::Second(state) => {
                    info!("websocket: processing state update");
                    self.send_state_via_ws(socket, state).await?;
                }
            }
        }
    }
}
