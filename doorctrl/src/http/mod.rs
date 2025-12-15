mod ascii;
pub mod header;
pub mod request;
pub mod response;
pub mod server;
pub mod websocket;

use embedded_io_async::Write;

pub(crate) trait HttpWrite {
    async fn write<T: Write>(self, writer: &mut T) -> Result<(), HTTPError>;
}

#[derive(Debug, defmt::Format, PartialEq)]
pub enum HTTPError {
    Incomplete,
    Disconnected,
    ProtocolError(&'static str),
    NetworkError(&'static str),
    UnsupportedRequest(&'static str),
    ExtraHeadersExceeded,
    WebsocketProtocolError,
}
