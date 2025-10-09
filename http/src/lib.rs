#![no_std]

pub mod response;

use core::str;
use defmt::Format;

const CR: u8 = 13;
const LF: u8 = 10;
const SP: u8 = 32;
const COLON: u8 = 58;
const ZERO: u8 = 48;

const GET: &'static [u8] = "GET".as_bytes();
const POST: &'static [u8] = "POST".as_bytes();
const PUT: &'static [u8] = "PUT".as_bytes();
const PATCH: &'static [u8] = "PATCH".as_bytes();
const DELETE: &'static [u8] = "DELETE".as_bytes();
const OPTIONS: &'static [u8] = "OPTIONS".as_bytes();

#[derive(Format, PartialEq, Debug)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    PATCH,
    DELETE,
    OPTIONS,
}

impl TryFrom<&[u8]> for HttpMethod {
    type Error = &'static str;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        match value {
            GET => Ok(Self::GET),
            POST => Ok(Self::POST),
            PUT => Ok(Self::PUT),
            PATCH => Ok(Self::PATCH),
            DELETE => Ok(Self::DELETE),
            OPTIONS => Ok(Self::OPTIONS),
            _ => Err("unknown http method"),
        }
    }
}

pub const UPGRADE: &'static str = "Upgrade";
pub const CONTENT_LENGTH: &'static str = "Content-Length";
pub const SEC_WEBSOCKET_KEY: &'static str = "Sec-WebSocket-Key";

#[derive(Debug, Copy, Clone)]
pub enum HttpHeader<'a> {
    ContentLength(u64),
    Upgrade(&'a str),
    SecWebSocketKey(&'a str),
    Null,
}

impl<'a> TryFrom<(&'a str, &'a str)> for HttpHeader<'a> {
    type Error = Option<&'static str>;

    fn try_from(value: (&'a str, &'a str)) -> Result<Self, Self::Error> {
        match value.0 {
            _ if value.0.eq_ignore_ascii_case(CONTENT_LENGTH) => Ok(HttpHeader::ContentLength(
                atoi(value.1.as_bytes())
                    .ok_or("invalid content-length")?
                    .into(),
            )),

            _ if value.0.eq_ignore_ascii_case(UPGRADE) => Ok(HttpHeader::Upgrade(value.1)),
            _ if value.0.eq_ignore_ascii_case(SEC_WEBSOCKET_KEY) => {
                Ok(HttpHeader::SecWebSocketKey(value.1))
            }
            _ => Err(None),
        }
    }
}

#[derive(Debug)]
pub enum HTTPError {
    NotReady,
    ProtocolErr(&'static str),
}

#[derive(Debug)]
pub struct HttpRequest<'a> {
    pub method: HttpMethod,
    pub path: &'a str,
    pub headers: [HttpHeader<'a>; 3],
}

impl<'a> TryFrom<&'a [u8]> for HttpRequest<'a> {
    type Error = HTTPError;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        let len = value.len();
        if len < 15 {
            // cant be a complete request...
            return Err(HTTPError::NotReady);
        }

        // search from offset for <CR><LF><CR><LF> which indicates the end of
        // headers
        for i in 1..len + 1 {
            match value[..i] {
                [.., CR, LF, CR, LF] => return Self::parse_request(&value[..i]),
                _ => {}
            };
        }

        return Err(HTTPError::NotReady);
    }
}

impl<'a> HttpRequest<'a> {
    fn parse_request(data: &'a [u8]) -> Result<Self, HTTPError> {
        // ensure upfront we have valid utf8 so later we can just unwrap str conversions
        if let Err(_) = str::from_utf8(data) {
            return Err(HTTPError::ProtocolErr("http request is not valid utf8"));
        }

        let mut req = HttpRequest {
            method: HttpMethod::GET,
            path: "",
            headers: [HttpHeader::Null; 3],
        };

        let mut request_line_done = false;

        let mut line_start = 0;
        for i in 0..data.len() {
            match &data[line_start..i] {
                [line @ .., CR, LF] => {
                    if !request_line_done {
                        req.parse_request_line(line)?;
                        request_line_done = true;
                    }
                    if request_line_done {
                        req.parse_header_line(line)?;
                    }
                    line_start = i;
                }
                _ => {}
            };
        }

        if req.path != "" {
            return Ok(req);
        }

        Err(HTTPError::ProtocolErr("malformed HTTP request"))
    }

    fn parse_request_line(&mut self, data: &'a [u8]) -> Result<(), HTTPError> {
        for (i, word) in data.splitn(3, |b: &u8| *b == SP).enumerate() {
            match i {
                0 => match HttpMethod::try_from(word) {
                    Ok(m) => self.method = m,
                    Err(_) => return Err(HTTPError::ProtocolErr("unknown http method")),
                },
                1 => self.path = str::from_utf8(word).unwrap(),
                2 => {}
                _ => return Err(HTTPError::ProtocolErr("malformed http request")),
            };
        }

        Ok(())
    }

    fn parse_header_line(&mut self, data: &'a [u8]) -> Result<(), HTTPError> {
        let mut header: &'a str = "";
        let mut value: &'a str;

        for (i, word) in data.splitn(2, |b: &u8| *b == COLON).enumerate() {
            match i {
                0 => {
                    header = str::from_utf8(word).unwrap().trim();
                }
                1 => {
                    value = str::from_utf8(word).unwrap().trim();
                    for (i, h) in self.headers.iter().enumerate() {
                        if let HttpHeader::Null = h {
                            match HttpHeader::try_from((header, value)) {
                                Ok(h) => {
                                    self.headers[i] = h;
                                    return Ok(());
                                }
                                Err(None) => {
                                    return Ok(());
                                }
                                Err(Some(e)) => {
                                    return Err(HTTPError::ProtocolErr(e));
                                }
                            }
                        }
                    }
                }
                _ => return Err(HTTPError::ProtocolErr("malformed http request")),
            };
        }
        Ok(())
    }

    pub fn content_len(&self) -> u64 {
        for h in self.headers {
            if let HttpHeader::ContentLength(n) = h {
                return n;
            }
        }

        return 0;
    }

    pub fn get_header(&self, name: &'static str) -> Option<&'a str> {
        for h in self.headers {
            match name {
                SEC_WEBSOCKET_KEY => {
                    if let HttpHeader::SecWebSocketKey(n) = h {
                        return Some(n);
                    }
                }
                UPGRADE => {
                    if let HttpHeader::Upgrade(n) = h {
                        return Some(n);
                    }
                }
                _ => {}
            }
        }
        None
    }
}

fn atoi(data: &[u8]) -> Option<u32> {
    let mut val: u32 = 0;

    let len: u32 = match data.len().try_into() {
        Ok(n) => n,
        Err(_) => return None,
    };

    for (i, digit) in data.iter().enumerate() {
        if *digit < 48 || *digit > 57 {
            return None;
        }

        let possition: u32 = match TryInto::<u32>::try_into(i) {
            Ok(n) => n + 1,
            Err(_) => return None,
        };

        let exp = len - possition;

        let digit_val: u32 = (digit - 48).into();
        val += digit_val * 10_u32.pow(exp);
    }

    return Some(val);
}

pub struct AsciiInt([u8; 20]);

impl AsciiInt {
    pub fn as_bytes(&self) -> &[u8] {
        &self.as_str().as_bytes()
    }

    pub fn as_str(&self) -> &str {
        str::from_utf8(&self.0).unwrap().trim()
    }
}

impl From<u64> for AsciiInt {
    fn from(value: u64) -> Self {
        let divmod10 = |d: u64| -> (u64, u8) {
            let int = d / 10;
            let rem = d % 10;
            (int, rem.try_into().unwrap())
        };

        let mut round = 0;
        let mut int = value;
        let mut rem: u8;

        let mut ret_array = [SP; 20];
        loop {
            (int, rem) = divmod10(int);
            ret_array[19 - round] = rem + ZERO;
            if int == 0 {
                break;
            }
            round += 1;
        }

        AsciiInt(ret_array)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn test_atoi() {
        assert!(atoi("0".as_bytes()) == Some(0));
        assert!(atoi("5".as_bytes()) == Some(5));
        assert!(atoi("123".as_bytes()) == Some(123));
        assert!(atoi("123456789".as_bytes()) == Some(123456789));
        assert!(atoi("0123456789".as_bytes()) == Some(123456789));
        assert!(atoi("abc".as_bytes()) == None);
        assert!(atoi("123a456".as_bytes()) == None);
    }

    #[test]
    fn test_itoa() {
        let a: AsciiInt = 1u64.into();
        assert!("1" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 12u64.into();
        assert!("12" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 123u64.into();
        assert!("123" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 1203u64.into();
        assert!("1203" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 12030u64.into();
        assert!("12030" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 100002u64.into();
        assert!("100002" == a.as_str(), "got: {:?}", a.as_str());
    }

    #[test]
    fn test_http_requrest_parsing_single_receive() {
        let req = "GET / HTTP/1.1\r\nContent-Length: 1234\r\n\r\n".as_bytes();

        let req = HttpRequest::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/");
        assert!(req.content_len() == 1234, "{:?}", req);

        let req = "GET /index.html HTTP/1.1\r\nContent-Length: 1234\r\n\r\n".as_bytes();

        let req = HttpRequest::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/index.html");
        assert!(req.content_len() == 1234, "{:?}", req);

        let req = "GET /index.html HTTP/1.1\r\ncontent-length: 1234\r\n\r\n".as_bytes();

        let req = HttpRequest::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/index.html");
        assert!(req.content_len() == 1234, "{:?}", req);

        let req = "GET /index.html HTTP/1.1\r\ncontent-type: application/json\r\ncontent-length: 1234\r\naccept-type: application/json\r\n\r\n".as_bytes();

        let req = HttpRequest::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/index.html");
        assert!(req.content_len() == 1234, "{:?}", req);
    }

    #[test]
    fn test_http_requrest_parsing_multiple_updates() {
        let mut http_buf = [0u8; 1024];
        let req_part_one = "GET / HTTP/1.1\r\nContentType:".as_bytes();
        let req_part_two = "application/json\r\n\r\n".as_bytes();

        http_buf[..req_part_one.len()].copy_from_slice(&req_part_one);
        http_buf[req_part_one.len()..req_part_one.len() + req_part_two.len()]
            .copy_from_slice(&req_part_two);

        let req = HttpRequest::try_from(&http_buf[..]).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/");
    }
}
