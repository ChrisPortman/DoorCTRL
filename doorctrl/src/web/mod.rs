use core::mem::size_of;
use core::str;

use defmt::{error, info, warn};
use embassy_futures::select;
use embassy_net::{tcp::TcpSocket, IpListenEndpoint, Stack};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Sender, pubsub::Subscriber,
};
use embedded_io_async::{Read, ReadExactError};

use crate::state::{AnyState, DoorState, LockState};

use http::{
    self,
    response::{respond, respond_websocket},
    HTTPError, HttpRequest,
};

const ERR_ACCEPT_ABORTED: &'static str = "waiting for connection aborted";

const WS_STATE_UPDATE: u8 = 1;
const WS_CONFIG_UPDATE: u8 = 2;

// state update payloads
const WS_LOCK_LOCK: u8 = 1;
const WS_LOCK_UNLOCK: u8 = 2;
const WS_DOOR_OPEN: u8 = 3;
const WS_DOOR_CLOSED: u8 = 4;

pub struct HttpService {}

impl HttpService {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn run<'a>(
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
                info!("processing request");
                let mut offset = 0;
                http_buf.fill(0);

                'read: loop {
                    // This loop handles the fact that a single read may not read an entire
                    // request.
                    info!("reading from socket");
                    match sock.read(&mut http_buf[offset..]).await {
                        Ok(0) => {
                            info!("http: connection closed by remote");
                            sock.close();
                            break 'request;
                        }
                        Ok(n) => {
                            offset += n;

                            match HttpRequest::try_from(&http_buf[..offset]) {
                                Ok(req) => {
                                    info!(
                                        "http: request to {} {} with {} bytes payload",
                                        req.method,
                                        req.path,
                                        req.content_len()
                                    );

                                    if req.content_len() > 0 {
                                        // We dont take requests with payloads, so rather than have
                                        // to handle syncronising the tcp stream by reading off the
                                        // content we dont want, we'll just disconnect the client
                                        // and start fresh.
                                        error!(
                                            "received request with payload which is unsupported"
                                        );
                                        _ = respond(400, &mut sock).await;
                                        sock.close();
                                        break 'request;
                                    }

                                    if let Err(e) = match req.method {
                                        http::HttpMethod::GET => Ok(()),
                                        _ => respond(400, &mut sock).await,
                                    } {
                                        error!("{}", e);
                                        sock.close();
                                        break 'request;
                                    }

                                    if let Err(e) = match req.path {
                                        "/" => respond(200, &mut sock).await,
                                        "/ws" => match req.get_header(http::UPGRADE) {
                                            None => Err("received /ws request without upgrade"),
                                            Some(u) if u == "websocket" => {
                                                if let Some(key) =
                                                    req.get_header(http::SEC_WEBSOCKET_KEY)
                                                {
                                                    respond_websocket(key, &mut sock).await?;
                                                    self.run_ws(
                                                        &mut sock,
                                                        &mut http_buf,
                                                        cmd_channel,
                                                        state_sub,
                                                    )
                                                    .await?;
                                                    sock.close();
                                                    break 'request;
                                                }
                                                Ok(())
                                            }
                                            _ => Err("unknown upgrade"),
                                        },
                                        _ => respond(404, &mut sock).await,
                                    } {
                                        error!("error processing request: {}", e);
                                        sock.close();
                                        break 'request;
                                    }

                                    break 'read;
                                }
                                Err(HTTPError::NotReady) => {
                                    info!("received partial, reading more");
                                    continue 'read;
                                }
                                Err(HTTPError::ProtocolErr(e)) => {
                                    error!("http: {}", e);
                                    sock.close();
                                    break 'request;
                                }
                            }
                        }
                        Err(e) => {
                            error!("http: error reading from socket: {}", e);
                            sock.close();
                            break 'request;
                        }
                    }
                }
            }
        }
    }

    pub async fn run_ws<'a, 'b>(
        &mut self,
        sock: &mut TcpSocket<'b>,
        buff: &mut [u8],
        cmd_channel: &Sender<'static, CriticalSectionRawMutex, LockState, 2>,
        state_sub: &mut Subscriber<'static, CriticalSectionRawMutex, AnyState, 2, 6, 0>,
    ) -> Result<(), &'static str> {
        //todo: websockets have a specific framing structure that we need to implement:
        // https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API/Writing_WebSocket_servers#exchanging_data_frames
        loop {
            info!("websocket processor waiting for state update or data from client");
            match select::select(
                sock.read_exact(&mut buff[..size_of::<u8>()]),
                state_sub.next_message_pure(),
            )
            .await
            {
                select::Either::First(Ok(())) => {
                    info!("processing client data");
                    match buff[0] {
                        WS_STATE_UPDATE => {
                            match sock.read_exact(&mut buff[..size_of::<u8>()]).await {
                                Ok(()) => match buff[0] {
                                    WS_LOCK_LOCK => cmd_channel.send(LockState::Locked).await,
                                    WS_LOCK_UNLOCK => cmd_channel.send(LockState::Unlocked).await,
                                    _ => warn!(
                                        "received unknown state update from websocket: {}",
                                        buff[0]
                                    ),
                                },
                                Err(ReadExactError::UnexpectedEof) => {
                                    info!("websocket: client closed websocket connection");
                                    return Ok(());
                                }
                                Err(ReadExactError::Other(e)) => {
                                    error!("websocket: error reading from connection: {}", e);
                                    return Err("websocket finished with error");
                                }
                            }
                        }
                        WS_CONFIG_UPDATE => {
                            // read a u32 size, then that number of bytes
                            // decode the config and store it, and reboot.
                        }
                        _ => {
                            error!("websocket: received unknown payload type: {}", buff[0]);
                            return Err("received unknown payload type");
                        }
                    }
                }
                select::Either::First(Err(ReadExactError::UnexpectedEof)) => {
                    info!("websocket: client closed websocket connection");
                    return Ok(());
                }
                select::Either::First(Err(ReadExactError::Other(e))) => {
                    error!("websocket: error reading from connection: {}", e);
                    return Err("websocket finished with error");
                }
                select::Either::Second(state) => {
                    info!("processing state update");
                    if let Err(e) = match state {
                        AnyState::LockState(LockState::Locked) => {
                            sock.write(&[WS_STATE_UPDATE, WS_LOCK_LOCK]).await
                        }
                        AnyState::LockState(LockState::Unlocked) => {
                            sock.write(&[WS_STATE_UPDATE, WS_LOCK_UNLOCK]).await
                        }
                        AnyState::DoorState(DoorState::Open) => {
                            sock.write(&[WS_STATE_UPDATE, WS_DOOR_OPEN]).await
                        }
                        AnyState::DoorState(DoorState::Closed) => {
                            sock.write(&[WS_STATE_UPDATE, WS_DOOR_CLOSED]).await
                        }
                    } {
                        error!("websocket: error writing to socket: {}", e);
                        return Err("error writing to websocket");
                    }
                }
            }
        }
    }
}
