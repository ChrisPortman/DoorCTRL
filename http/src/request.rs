use core::mem::discriminant;
use core::usize;
use defmt::Format;

use crate::HTTPError;
use crate::ascii::{COLON, CR, LF, SP};
use crate::header::HttpHeader;

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

#[derive(Debug, Format)]
pub struct HttpRequest<'a, const MAX_EXTRA_HEADERS: usize> {
    pub method: HttpMethod,
    pub path: &'a str,
    pub body: Option<&'a [u8]>,
    content_length: Option<HttpHeader<'a>>,
    headers: [Option<HttpHeader<'a>>; MAX_EXTRA_HEADERS],
}

impl<'a, const MAX_EXTRA_HEADERS: usize> TryFrom<&'a [u8]> for HttpRequest<'a, MAX_EXTRA_HEADERS> {
    type Error = HTTPError;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        let len = value.len();
        if len < 15 {
            // cant be a complete request...
            return Err(HTTPError::Incomplete);
        }

        // search from offset for <CR><LF><CR><LF> which indicates the end of
        // headers
        for i in 1..len + 1 {
            match value[..i] {
                [.., CR, LF, CR, LF] => return Self::parse_request(&value[..i]),
                _ => {}
            };
        }

        return Err(HTTPError::Incomplete);
    }
}

impl<'a, const MAX_EXTRA_HEADERS: usize> HttpRequest<'a, MAX_EXTRA_HEADERS> {
    pub fn contains_request_headers(data: &[u8]) -> Option<usize> {
        let len = data.len();

        for i in 1..len + 1 {
            match data[..i] {
                [.., CR, LF, CR, LF] => return Some(i),
                _ => {}
            };
        }

        None
    }

    pub fn parse_request(data: &'a [u8]) -> Result<Self, HTTPError> {
        // ensure upfront we have valid utf8 so later we can just unwrap str conversions
        if let Err(_) = str::from_utf8(data) {
            return Err(HTTPError::ProtocolError("http request is not valid utf8"));
        }

        let mut req = HttpRequest {
            method: HttpMethod::GET,
            path: "",
            content_length: None,
            headers: [None; MAX_EXTRA_HEADERS],
            body: None,
        };

        let mut request_line_done = false;

        let mut line_start = 0;
        for i in 0..data.len() {
            match &data[line_start..i] {
                [line @ .., CR, LF] => {
                    if !request_line_done {
                        req.parse_request_line(line)?;
                        request_line_done = true;
                    } else {
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

        Err(HTTPError::ProtocolError("malformed HTTP request"))
    }

    fn parse_request_line(&mut self, data: &'a [u8]) -> Result<(), HTTPError> {
        for (i, word) in data.splitn(3, |b: &u8| *b == SP).enumerate() {
            match i {
                0 => match HttpMethod::try_from(word) {
                    Ok(m) => self.method = m,
                    Err(_) => return Err(HTTPError::ProtocolError("unknown http method")),
                },
                1 => self.path = str::from_utf8(word).unwrap(),
                2 => {}
                _ => return Err(HTTPError::ProtocolError("malformed http request")),
            };
        }

        Ok(())
    }

    fn parse_header_line(&mut self, data: &'a [u8]) -> Result<(), HTTPError> {
        let mut header: Option<&'a str> = None;
        let mut value: Option<&'a str> = None;
        let mut slot: Option<usize> = None;

        for (i, word) in data.splitn(2, |b: &u8| *b == COLON).enumerate() {
            match i {
                0 => {
                    header = Some(str::from_utf8(word).unwrap().trim());
                }
                1 => {
                    value = Some(str::from_utf8(word).unwrap().trim());
                }
                _ => return Err(HTTPError::ProtocolError("malformed http request")),
            }
        }

        for (i, h) in self.headers.iter().enumerate() {
            if let None = h {
                slot = Some(i);
                break;
            }
        }

        if let Some(header) = header
            && let Some(value) = value
        {
            match HttpHeader::try_from((header, value)) {
                Ok(h) => {
                    if let HttpHeader::ContentLength(_) = h {
                        self.content_length = Some(h);
                        return Ok(());
                    }

                    if let Some(s) = slot {
                        self.headers[s] = Some(h);
                    }
                    return Ok(());
                }
                Err(None) => {
                    return Ok(());
                }
                Err(Some(e)) => {
                    return Err(HTTPError::ProtocolError(e));
                }
            }
        }

        Ok(())
    }

    pub fn content_len(&self) -> usize {
        if let Some(HttpHeader::ContentLength(n)) = self.content_length {
            return n;
        }

        return 0;
    }

    pub fn get_header(&self, head: HttpHeader) -> Option<&HttpHeader<'a>> {
        for h in &self.headers {
            match h {
                Some(h) => {
                    if discriminant(h) == discriminant(&head) {
                        if matches!(
                            (h, head),
                            (HttpHeader::Other(k1, _), HttpHeader::Other(k2,_ )) if *k1 == k2
                        ) {
                            // The requested header was *Other* so match if the requested key field
                            // matches.
                            return Some(h);
                        }
                        return Some(h);
                    }
                }
                None => return None,
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn test_http_request_parsing_single_receive() {
        let req = "GET / HTTP/1.1\r\nContent-Length: 1234\r\n\r\n".as_bytes();

        let req = HttpRequest::<3>::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/");
        assert!(req.content_len() == 1234, "{:?}", req);

        let req = "GET /index.html HTTP/1.1\r\nContent-Length: 1234\r\n\r\n".as_bytes();

        let req = HttpRequest::<3>::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/index.html");
        assert!(req.content_len() == 1234, "{:?}", req);

        let req = "GET /index.html HTTP/1.1\r\ncontent-length: 1234\r\n\r\n".as_bytes();

        let req = HttpRequest::<3>::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/index.html");
        assert!(req.content_len() == 1234, "{:?}", req);

        let req = "GET /index.html HTTP/1.1\r\ncontent-type: application/json\r\ncontent-length: 1234\r\naccept-type: application/json\r\n\r\n".as_bytes();

        let req = HttpRequest::<3>::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/index.html");
        assert!(req.content_len() == 1234, "{:?}", req);
        std::println!("{:?}", req);

        assert_eq!(
            req.get_header(HttpHeader::Other("content-type", "")),
            Some(&HttpHeader::Other("content-type", "application/json"))
        );
    }

    #[test]
    fn test_http_request_parsing_multiple_updates() {
        let mut http_buf = [0u8; 1024];
        let req_part_one = "GET / HTTP/1.1\r\nContentType:".as_bytes();
        let req_part_two = "application/json\r\n\r\n".as_bytes();

        http_buf[..req_part_one.len()].copy_from_slice(&req_part_one);
        http_buf[req_part_one.len()..req_part_one.len() + req_part_two.len()]
            .copy_from_slice(&req_part_two);

        let req = HttpRequest::<3>::try_from(&http_buf[..]).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/");
    }
}
