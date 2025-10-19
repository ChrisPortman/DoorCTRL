use defmt::Format;
use embedded_io_async::Write;

use crate::ascii::{AsciiInt, CR, LF, atoi};
use crate::{HTTPError, HttpWrite};

pub const HTTP_HEAD_UPGRADE: &'static str = "Upgrade";
pub const HTTP_HEAD_CONTENT_LENGTH: &'static str = "Content-Length";
pub const HTTP_HEAD_SEC_WEBSOCKET_KEY: &'static str = "Sec-WebSocket-Key";

#[derive(Clone, Copy, Debug, PartialEq, Format)]
pub enum HttpHeader<'a> {
    ContentType(&'a str),
    ContentLength(usize),
    Server(&'a str),
    Upgrade(&'a str),
    SecWebSocketKey(&'a str),
    SecWebSocketAccept([u8; 28]),
    Other(&'a str, &'a str),
}

impl<'a> HttpWrite for HttpHeader<'a> {
    #[rustfmt::skip]
    async fn write<T: Write>(self, writer: &mut T) -> Result<(), HTTPError> {
        let len: AsciiInt;
        let ws_accept: [u8;28];

        let val = match self {
            Self::ContentType(s) => {
                writer.write_all("Content-Type: ".as_bytes()).await.or(Err(HTTPError::NetworkError("connnection reset by peer")))?;
                s
            },
            Self::ContentLength(n)=> {
                if n == 0 {
                    return Ok(());
                }

                writer.write_all("Content-Length: ".as_bytes()).await.or(Err(HTTPError::NetworkError("connnection reset by peer")))?;
                len = AsciiInt::from(n as u64);
                len.as_str()
            },
            Self::Server(s)=> {
                writer.write_all("Server: ".as_bytes()).await.or(Err(HTTPError::NetworkError("connnection reset by peer")))?;
                s
            },
            Self::SecWebSocketAccept(s) => {
                writer.write_all("Sec-WebSocket-Accept: ".as_bytes()).await.or(Err(HTTPError::NetworkError("connnection reset by peer")))?;
                ws_accept = s;
                str::from_utf8(&ws_accept).unwrap()
            },
            Self::Other(k, v) => {
                writer.write_all(k.as_bytes()).await.and(writer.write(": ".as_bytes()).await).or(Err(HTTPError::NetworkError("connnection reset by peer")))?;
                v
            },
            _ => return Err(HTTPError::ProtocolError("invalid response header"))
        };

        writer.write_all(val.as_bytes()).await.and(writer.write_all(&[CR, LF]).await).or(Err(HTTPError::NetworkError("connnection reset by peer")))
    }
}

impl<'a> TryFrom<(&'a str, &'a str)> for HttpHeader<'a> {
    type Error = Option<&'static str>;

    fn try_from(value: (&'a str, &'a str)) -> Result<Self, Self::Error> {
        match value.0 {
            _ if value.0.eq_ignore_ascii_case(HTTP_HEAD_CONTENT_LENGTH) => {
                Ok(HttpHeader::ContentLength(
                    atoi(value.1.as_bytes()).ok_or("invalid content-length")? as usize,
                ))
            }

            _ if value.0.eq_ignore_ascii_case(HTTP_HEAD_UPGRADE) => {
                Ok(HttpHeader::Upgrade(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(HTTP_HEAD_SEC_WEBSOCKET_KEY) => {
                Ok(HttpHeader::SecWebSocketKey(value.1))
            }
            _ => Ok(HttpHeader::Other(value.0, value.1)),
        }
    }
}
