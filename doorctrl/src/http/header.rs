use defmt::Format;
use embedded_io_async::Write;

use crate::http::ascii::{AsciiInt, CR, LF, atoi};
use crate::http::{HTTPError, HttpWrite};

pub const REQ_HEAD_HOST: &str = "Host";
pub const REQ_HEAD_USER_AGENT: &str = "User-Agent";
pub const REQ_HEAD_UPGRADE: &str = "Upgrade";
pub const REQ_HEAD_SEC_WEBSOCKET_KEY: &str = "Sec-WebSocket-Key";
pub const REQ_HEAD_ACCEPT: &str = "Accept";
pub const REQ_HEAD_ACCEPT_LANGUAGE: &str = "Accept-Language";
pub const REQ_HEAD_ACCEPT_ENCODING: &str = "Accept-Encoding";
pub const REQ_HEAD_REFERER: &str = "Referer";
pub const REQ_HEAD_CONNECTION: &str = "Connection";
pub const REQ_HEAD_UPGRADE_INSECURE_REQUESTS: &str = "Upgrade-Insecure-Requests";
pub const REQ_HEAD_IF_MODIFIED_SINCE: &str = "If-Modified-Since";
pub const REQ_HEAD_IF_NONE_MATCH: &str = "If-None-Match";
pub const REQ_HEAD_CACHE_CONTROL: &str = "Cache-Control";
pub const REQ_HEAD_CONTENT_LENGTH: &str = "Content-Length";
pub const REQ_HEAD_CONTENT_RANGE: &str = "Content-Range";
pub const REQ_HEAD_CONTENT_TYPE: &str = "Content-Type";
pub const REQ_HEAD_CONTENT_ENCODING: &str = "Content-Encoding";
pub const REQ_HEAD_CONTENT_LOCATION: &str = "Content-Location";
pub const REQ_HEAD_CONTENT_LANGUAGE: &str = "Content-Language";
pub const REQ_HEAD_ETAG: &str = "ETag";

#[derive(Clone, Copy, Debug, PartialEq, Format)]
pub enum RequestHeader<'a> {
    Host(&'a str),
    UserAgent(&'a str),
    Upgrade(&'a str),
    SecWebSocketKey(&'a str),
    Accept(&'a str),
    AcceptLanguage(&'a str),
    AcceptEncoding(&'a str),
    Referer(&'a str),
    Connection(&'a str),
    UpgradeInsecureRequests(&'a str),
    IfModifiedSince(&'a str),
    IfNoneMatch(&'a str),
    CacheControl(&'a str),
    ContentLength(usize),
    ContentRange(&'a str),
    ContentType(&'a str),
    ContentEncoding(&'a str),
    ContentLocation(&'a str),
    ContentLanguage(&'a str),
    ETag(&'a str),
    Other(&'a str, &'a str),
}

impl<'a> TryFrom<(&'a str, &'a str)> for RequestHeader<'a> {
    type Error = Option<&'static str>;

    fn try_from(value: (&'a str, &'a str)) -> Result<Self, Self::Error> {
        match value.0 {
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_HOST) => Ok(RequestHeader::Host(value.1)),
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_USER_AGENT) => {
                Ok(RequestHeader::UserAgent(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_UPGRADE) => {
                Ok(RequestHeader::Upgrade(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_SEC_WEBSOCKET_KEY) => {
                Ok(RequestHeader::SecWebSocketKey(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_ACCEPT) => {
                Ok(RequestHeader::Accept(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_ACCEPT_LANGUAGE) => {
                Ok(RequestHeader::AcceptLanguage(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_ACCEPT_ENCODING) => {
                Ok(RequestHeader::AcceptEncoding(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_REFERER) => {
                Ok(RequestHeader::Referer(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_CONNECTION) => {
                Ok(RequestHeader::Connection(value.1))
            }
            _ if value
                .0
                .eq_ignore_ascii_case(REQ_HEAD_UPGRADE_INSECURE_REQUESTS) =>
            {
                Ok(RequestHeader::UpgradeInsecureRequests(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_IF_MODIFIED_SINCE) => {
                Ok(RequestHeader::IfModifiedSince(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_IF_NONE_MATCH) => {
                Ok(RequestHeader::IfNoneMatch(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_CACHE_CONTROL) => {
                Ok(RequestHeader::CacheControl(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_CONTENT_RANGE) => {
                Ok(RequestHeader::ContentRange(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_CONTENT_TYPE) => {
                Ok(RequestHeader::ContentType(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_CONTENT_ENCODING) => {
                Ok(RequestHeader::ContentEncoding(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_CONTENT_LOCATION) => {
                Ok(RequestHeader::ContentLocation(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_CONTENT_LANGUAGE) => {
                Ok(RequestHeader::ContentLanguage(value.1))
            }
            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_ETAG) => Ok(RequestHeader::ETag(value.1)),

            _ if value.0.eq_ignore_ascii_case(REQ_HEAD_CONTENT_LENGTH) => {
                Ok(RequestHeader::ContentLength(
                    atoi(value.1.as_bytes()).ok_or("invalid content-length")? as usize,
                ))
            }
            _ => Ok(RequestHeader::Other(value.0, value.1)),
        }
    }
}

pub const RESP_HEAD_ACCESS_CONTROL_ALLOW_ORIGIN: &str = "Access-Control-Allow-Origin";
pub const RESP_HEAD_CONNECTION: &str = "Connection";
pub const RESP_HEAD_DATE: &str = "Date";
pub const RESP_HEAD_KEEP_ALIVE: &str = "Keep-Alive";
pub const RESP_HEAD_LAST_MODIFIED: &str = "Last-Modified";
pub const RESP_HEAD_SERVER: &str = "Server";
pub const RESP_HEAD_SET_COOKIE: &str = "Set-Cookie";
pub const RESP_HEAD_TRANSFER_ENCODING: &str = "Transfer-Encoding";
pub const RESP_HEAD_VARY: &str = "Vary";
pub const RESP_HEAD_CONTENT_LENGTH: &str = "Content-Length";
pub const RESP_HEAD_CONTENT_RANGE: &str = "Content-Range";
pub const RESP_HEAD_CONTENT_TYPE: &str = "Content-Type";
pub const RESP_HEAD_CONTENT_ENCODING: &str = "Content-Encoding";
pub const RESP_HEAD_CONTENT_LOCATION: &str = "Content-Location";
pub const RESP_HEAD_CONTENT_LANGUAGE: &str = "Content-Language";
pub const RESP_HEAD_ETAG: &str = "ETag";
pub const RESP_HEAD_SEC_WEBSOCKET_ACCEPT: &str = "Sec-WebSocket-Accept";

#[derive(Clone, Copy, Debug, PartialEq, Format)]
pub enum ResponseHeader<'a> {
    AccessControlAllowOrigin(&'a str),
    Connection(&'a str),
    Date(&'a str),
    KeepAlive(&'a str),
    LastModified(&'a str),
    Server(&'a str),
    SetCookie(&'a str),
    TransferEncoding(&'a str),
    Vary(&'a str),
    ContentLength(usize),
    ContentRange(&'a str),
    ContentType(&'a str),
    ContentEncoding(&'a str),
    ContentLocation(&'a str),
    ContentLanguage(&'a str),
    ETag(&'a str),
    SecWebSocketAccept([u8; 28]),
    Other(&'a str, &'a str),
}

impl<'a> HttpWrite for ResponseHeader<'a> {
    async fn write<T: Write>(self, writer: &mut T) -> Result<(), HTTPError> {
        let len: AsciiInt;
        let ws_accept: [u8; 28];

        let val = match self {
            Self::AccessControlAllowOrigin(s) => {
                writer
                    .write_all(RESP_HEAD_ACCESS_CONTROL_ALLOW_ORIGIN.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::Connection(s) => {
                writer
                    .write_all(RESP_HEAD_CONNECTION.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::Date(s) => {
                writer
                    .write_all(RESP_HEAD_DATE.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::KeepAlive(s) => {
                writer
                    .write_all(RESP_HEAD_KEEP_ALIVE.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::LastModified(s) => {
                writer
                    .write_all(RESP_HEAD_LAST_MODIFIED.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::Server(s) => {
                writer
                    .write_all(RESP_HEAD_SERVER.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::SetCookie(s) => {
                writer
                    .write_all(RESP_HEAD_SET_COOKIE.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::TransferEncoding(s) => {
                writer
                    .write_all(RESP_HEAD_TRANSFER_ENCODING.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::Vary(s) => {
                writer
                    .write_all(RESP_HEAD_VARY.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::ContentLength(n) => {
                if n == 0 {
                    return Ok(());
                }
                writer
                    .write_all(RESP_HEAD_CONTENT_LENGTH.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;

                len = AsciiInt::from(n as u64);
                len.as_str()
            }
            Self::ContentRange(s) => {
                writer
                    .write_all(RESP_HEAD_CONTENT_RANGE.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::ContentType(s) => {
                writer
                    .write_all(RESP_HEAD_CONTENT_TYPE.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::ContentEncoding(s) => {
                writer
                    .write_all(RESP_HEAD_CONTENT_ENCODING.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::ContentLocation(s) => {
                writer
                    .write_all(RESP_HEAD_CONTENT_LOCATION.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::ContentLanguage(s) => {
                writer
                    .write_all(RESP_HEAD_CONTENT_LANGUAGE.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::ETag(s) => {
                writer
                    .write_all(RESP_HEAD_ETAG.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                s
            }
            Self::SecWebSocketAccept(s) => {
                writer
                    .write_all(RESP_HEAD_SEC_WEBSOCKET_ACCEPT.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                ws_accept = s;
                str::from_utf8(&ws_accept).unwrap()
            }
            Self::Other(k, v) => {
                writer
                    .write_all(k.as_bytes())
                    .await
                    .or(Err(HTTPError::Disconnected))?;
                v
            }
        };

        writer
            .write_all(": ".as_bytes())
            .await
            .and(writer.write_all(val.as_bytes()).await)
            .and(writer.write_all(&[CR, LF]).await)
            .or(Err(HTTPError::Disconnected))
    }
}
