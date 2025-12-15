use core::mem::discriminant;
use defmt::Format;

use crate::http::HTTPError;
use crate::http::ascii::{COLON, CR, LF, SP};
use crate::http::header::RequestHeader;

const GET: &[u8] = "GET".as_bytes();
const POST: &[u8] = "POST".as_bytes();
const PUT: &[u8] = "PUT".as_bytes();
const PATCH: &[u8] = "PATCH".as_bytes();
const DELETE: &[u8] = "DELETE".as_bytes();
const OPTIONS: &[u8] = "OPTIONS".as_bytes();
const HEAD: &[u8] = "HEAD".as_bytes();

#[derive(Format, PartialEq, Debug)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    PATCH,
    DELETE,
    OPTIONS,
    HEAD,
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
            HEAD => Ok(Self::HEAD),
            _ => Err("unknown http method"),
        }
    }
}

pub enum RequestBody<'a> {
    Complete(&'a [u8]),
    Partial((usize, &'a [u8])),
    None,
}

#[derive(Debug, Format)]
pub struct HttpRequest<'a> {
    pub method: HttpMethod,
    pub path: &'a str,
    pub host: &'a str,
    pub content_type: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub content_length: usize,
    pub(crate) body: Option<&'a [u8]>,
    pub(crate) header_slice: Option<&'a [u8]>,
}

impl<'a> TryFrom<&'a [u8]> for HttpRequest<'a> {
    type Error = HTTPError;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        let len = value.len();
        if len < 15 {
            // cant be a complete request...
            return Err(HTTPError::Incomplete);
        }

        Self::parse(value)
    }
}

impl<'a> HttpRequest<'a> {
    pub fn contains_complete_http_header(data: &[u8]) -> Option<usize> {
        let len = data.len();

        for i in 1..len + 1 {
            if let [.., CR, LF, CR, LF] = data[..i] {
                return Some(i);
            }
        }

        None
    }

    pub fn parse(data: &'a [u8]) -> Result<Self, HTTPError> {
        // ensure upfront we have valid utf8 so later we can just unwrap str conversions
        if str::from_utf8(data).is_err() {
            return Err(HTTPError::ProtocolError("http request is not valid utf8"));
        }

        let mut req = HttpRequest {
            method: HttpMethod::GET,
            path: "",
            host: "unspecified",
            content_type: None,
            user_agent: None,
            content_length: 0,
            header_slice: None,
            body: None,
        };

        let mut request_line_done = false;
        let mut http_headers_done = false;
        let mut header_start_offset = 0usize;
        let mut header_end_offset = 0usize;

        let mut line_start = 0;
        for i in 0..=data.len() {
            if let [CR, LF] = &data[line_start..i] {
                // a \r\n imediately after a line\r\n indicates the end of the headers
                http_headers_done = true;

                if req.content_length > 0 {
                    req.body = data.get(i..i + req.content_length);
                    if req.body.is_none() {
                        return Err(HTTPError::Incomplete);
                    }
                }

                break;
            }

            if let [line @ .., CR, LF] = &data[line_start..i] {
                if !request_line_done {
                    req.parse_request_line(line)?;
                    request_line_done = true;
                } else {
                    req.parse_header_line(line)?;
                    if header_start_offset == 0 {
                        header_start_offset = line_start;
                    }
                    header_end_offset = i;
                }
                line_start = i;
            }
        }

        if header_start_offset != 0 && header_end_offset != 0 {
            req.header_slice = Some(&data[header_start_offset..header_end_offset])
        }

        if !http_headers_done {
            return Err(HTTPError::Incomplete);
        }

        if req.path.is_empty() {
            return Err(HTTPError::ProtocolError("malformed HTTP request"));
        }

        Ok(req)
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

        if let Some(header) = header
            && let Some(value) = value
        {
            match RequestHeader::try_from((header, value)) {
                Ok(h) => {
                    if let RequestHeader::ContentLength(l) = h {
                        self.content_length = l;
                        return Ok(());
                    }
                    if let RequestHeader::Host(s) = h {
                        self.host = s;
                        return Ok(());
                    }
                    if let RequestHeader::ContentType(s) = h {
                        self.content_type = Some(s);
                        return Ok(());
                    }
                    if let RequestHeader::UserAgent(s) = h {
                        self.user_agent = Some(s);
                        return Ok(());
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

    fn resolve_header(&self, data: &'a [u8]) -> Result<Option<RequestHeader<'a>>, HTTPError> {
        let mut header: Option<&'a str> = None;
        let mut value: Option<&'a str> = None;

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

        if let Some(header) = header
            && let Some(value) = value
        {
            match RequestHeader::try_from((header, value)) {
                Ok(h) => {
                    return Ok(Some(h));
                }
                Err(None) => {
                    return Ok(None);
                }
                Err(Some(e)) => {
                    return Err(HTTPError::ProtocolError(e));
                }
            }
        }

        Ok(None)
    }

    pub fn get_header(&self, header: RequestHeader<'_>) -> Option<RequestHeader<'a>> {
        if let Some(data) = self.header_slice {
            let mut line_start = 0;

            for i in 0..=data.len() {
                if let [line @ .., CR, LF] = &data[line_start..i] {
                    if let Ok(Some(h)) = self.resolve_header(line) {
                        match (header, h) {
                            (RequestHeader::Other(key1, _), RequestHeader::Other(key2, _))
                                if key1.eq_ignore_ascii_case(key2) =>
                            {
                                return Some(h);
                            }
                            (RequestHeader::Other(_, _), RequestHeader::Other(_, _)) => {}
                            (h1, h2) if discriminant(&h1) == discriminant(&h2) => {
                                return Some(h);
                            }
                            _ => {}
                        };
                    }
                    line_start = i;
                }
            }
        };

        None
    }

    pub fn get_body(&self) -> RequestBody<'a> {
        match self.body {
            None => RequestBody::None,
            Some(b) => {
                if b.len() < self.content_length {
                    RequestBody::Partial((self.content_length - b.len(), b))
                } else {
                    RequestBody::Complete(b)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn test_http_request_parsing_single_receive() {
        let req = "GET / HTTP/1.1\r\nContent-Length: 0\r\n\r\n".as_bytes();

        let req = HttpRequest::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/");
        assert!(req.content_length == 0, "{:?}", req);

        let req = "GET /index.html HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc".as_bytes();

        let req = HttpRequest::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/index.html");
        assert!(req.content_length == 3, "{:?}", req);
        assert_eq!(req.body, Some("abc".as_bytes()));

        let req = "GET /index.html HTTP/1.1\r\ncontent-type: application/json\r\ncontent-length: 3\r\naccept: application/json\r\nAccept-Encoding: gzip\r\n\r\nabc".as_bytes();

        let req = HttpRequest::try_from(req).unwrap();
        assert!(req.method == HttpMethod::GET);
        assert!(req.path == "/index.html");
        assert!(req.content_length == 3, "{:?}", req);
        assert_eq!(req.content_type, Some("application/json"));
        assert_eq!(
            req.get_header(RequestHeader::ContentType("")),
            Some(RequestHeader::ContentType("application/json"))
        );
        assert_eq!(
            req.get_header(RequestHeader::AcceptEncoding("")),
            Some(RequestHeader::AcceptEncoding("gzip"))
        );
        assert_eq!(
            req.get_header(RequestHeader::Accept("")),
            Some(RequestHeader::Accept("application/json"))
        );
        assert_eq!(req.body, Some("abc".as_bytes()));
    }

    #[test]
    fn test_http_request_parsing_multiple_updates() {
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
