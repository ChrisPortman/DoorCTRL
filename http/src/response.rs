use core::mem::discriminant;
use embedded_io_async::Write;

use crate::ascii::{AsciiInt, CR, LF, SP};
use crate::header::HttpHeader;
use crate::{HTTPError, HttpWrite};

const HTTP_PROTO: &str = "HTTP/1.1";

#[derive(Clone, Copy)]
pub enum HttpStatusCode {
    SwitchingProtocols,
    OK,
    BadRequest,
    NotFound,
    InternalServerError,
    Other(u16),
}

impl HttpWrite for HttpStatusCode {
    #[rustfmt::skip]
    async fn write<T: Write>(self, writer: &mut T) -> Result<(), HTTPError> {
        let other: AsciiInt;
        let data = match self {
            Self::SwitchingProtocols => "101 Switching Protocols",
            Self::OK => "200 OK",
            Self::BadRequest => "400 Bad Request",
            Self::NotFound => "404 Not Found",
            Self::InternalServerError => "500 Internal Server Error",
            Self::Other(n) => {
                if n <100 || n > 599 {
                    return Err(HTTPError::ProtocolError("invalid status code"));
                }
                other = AsciiInt::from(n as u64);
                other.as_str()
            }
        };

        writer.write_all(HTTP_PROTO.as_bytes()).await
            .and(writer.write_all(&[SP]).await
            .and(writer.write_all(data.as_bytes()).await
            .and(writer.write_all(&[CR, LF]).await
        ))).or(Err(HTTPError::NetworkError("connnection reset by peer")))
    }
}

pub struct HttpResponse<'a, const MAX_EXTRA_HEADERS: usize> {
    status_code: HttpStatusCode,
    server: HttpHeader<'a>,
    content_type: HttpHeader<'a>,
    content_length: HttpHeader<'a>,
    extra_headers: [Option<HttpHeader<'a>>; MAX_EXTRA_HEADERS],
}

impl<'a, const MAX_EXTRA_HEADERS: usize> HttpResponse<'a, MAX_EXTRA_HEADERS> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_status(&self) -> HttpStatusCode {
        self.status_code
    }

    pub fn set_status(&mut self, status: HttpStatusCode) {
        self.status_code = status
    }

    pub fn set_server(&mut self, server: &'a str) {
        self.server = HttpHeader::Server(server)
    }

    pub fn set_content_type(&mut self, ct: &'a str) {
        self.content_type = HttpHeader::ContentType(ct)
    }

    pub fn add_extra_header(&mut self, header: HttpHeader<'a>) -> Result<(), HTTPError> {
        let mut slot: Option<usize> = None;

        for (i, existing) in self.extra_headers.iter().enumerate() {
            match existing {
                Some(existing) => {
                    if discriminant(existing) == discriminant(&header) {
                        if matches!((existing, header), (HttpHeader::Other(ek, _), HttpHeader::Other(nk, _)) if *ek != nk)
                        {
                            // Both headers are custom, but for different custom headers
                            continue;
                        }
                        self.extra_headers[i] = Some(header);
                        return Ok(());
                    }
                }
                None => {
                    if let None = slot {
                        slot = Some(i);
                    }
                }
            };
        }

        if let Some(slot) = slot {
            self.extra_headers[slot] = Some(header);
            return Ok(());
        }

        Err(HTTPError::ExtraHeadersExceeded)
    }


    #[rustfmt::skip]
    pub async fn send<T: Write>(self, writer: &mut T) -> Result<(), HTTPError> {
        self.status_code.write(writer).await
            .and(self.server.write(writer).await)
            .and(self.content_type.write(writer).await)?;

        if let HttpHeader::ContentLength(1..) = self.content_length {
            self.content_length.write(writer).await?;
        }

        for head in self.extra_headers {
            if let Some(head) = head {
                 head.write(writer).await?;
            }
        }

        writer.write_all(&[CR, LF]).await
            .or(Err(HTTPError::NetworkError("connection reset by peer")))?;

        Ok(())
    }

    pub async fn send_with_body<T: Write>(
        mut self,
        writer: &mut T,
        body: &[u8],
    ) -> Result<(), HTTPError> {
        self.content_length = HttpHeader::ContentLength(body.len());
        self.send(writer)
            .await
            .or(Err(HTTPError::NetworkError("connection closed by peer")))?;

        writer
            .write_all(body)
            .await
            .or(Err(HTTPError::NetworkError("connection closed by peer")))?;

        Ok(())
    }
}

impl<'a, const MAX_EXTRA_HEADERS: usize> Default for HttpResponse<'a, MAX_EXTRA_HEADERS> {
    fn default() -> Self {
        Self {
            status_code: HttpStatusCode::OK,
            server: HttpHeader::Server("RustServer"),
            content_type: HttpHeader::ContentType("text/html"),
            content_length: HttpHeader::ContentLength(0),
            extra_headers: [None; MAX_EXTRA_HEADERS],
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use embedded_io_async::{ErrorKind, ErrorType};
    use std::vec::Vec;
    use std::*;

    use super::*;

    struct TestWriter<'a> {
        inner: &'a mut Vec<u8>,
    }

    impl<'a> TestWriter<'a> {
        fn new(inner: &'a mut Vec<u8>) -> Self {
            Self { inner: inner }
        }
    }

    impl<'a> ErrorType for TestWriter<'a> {
        type Error = ErrorKind;
    }

    impl<'a> Write for TestWriter<'a> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.inner.extend_from_slice(buf);
            Ok(buf.len())
        }

        async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
            self.inner.extend_from_slice(buf);
            Ok(())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    // HTTP uses `\r\n` as EOL delimeters.  In the expected data, we manually add
    // the \r at the end of the line, before the inherrent \n.

    #[tokio::test]
    async fn test_http_response_default() {
        let resp = HttpResponse::<3>::new();
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
\r
"
        .as_bytes();

        if let Err(e) = resp.send(&mut writer).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_default_with_body() {
        let resp = HttpResponse::<3>::new();
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        let body = "<html>
    <head>
    <title>Testing</title>
    </head>
    <body>
        <p>works!</p>
    </body>
</html>
"
        .as_bytes();

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
Content-Length: 110\r
\r
<html>
    <head>
    <title>Testing</title>
    </head>
    <body>
        <p>works!</p>
    </body>
</html>
"
        .as_bytes();

        if let Err(e) = resp.send_with_body(&mut writer, body).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_status() {
        let mut resp = HttpResponse::<3>::new();
        resp.set_status(HttpStatusCode::NotFound);
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        let expected = "HTTP/1.1 404 Not Found\r
Server: RustServer\r
Content-Type: text/html\r
\r
"
        .as_bytes();

        if let Err(e) = resp.send(&mut writer).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_custom_status() {
        let mut resp = HttpResponse::<3>::new();
        resp.set_status(HttpStatusCode::Other(401));
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        let expected = "HTTP/1.1 401\r
Server: RustServer\r
Content-Type: text/html\r
\r
"
        .as_bytes();

        if let Err(e) = resp.send(&mut writer).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_custom_content_type() {
        let mut resp = HttpResponse::<3>::new();
        resp.set_content_type("application/json");
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: application/json\r
\r
"
        .as_bytes();

        if let Err(e) = resp.send(&mut writer).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_custom_server() {
        let mut resp = HttpResponse::<3>::new();
        resp.set_server("FancyServer");
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        let expected = "HTTP/1.1 200 OK\r
Server: FancyServer\r
Content-Type: text/html\r
\r
"
        .as_bytes();

        if let Err(e) = resp.send(&mut writer).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_one_extra_header() {
        let mut resp = HttpResponse::<3>::new();
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo", "Bar")) {
            self::panic!("{:?}", e);
        }

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
Foo: Bar\r
\r
"
        .as_bytes();

        if let Err(e) = resp.send(&mut writer).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_replaced_extra_header() {
        let mut resp = HttpResponse::<3>::new();
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo", "Bar")) {
            self::panic!("{:?}", e);
        }
        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo", "Baz")) {
            self::panic!("{:?}", e);
        }

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
Foo: Baz\r
\r
"
        .as_bytes();

        if let Err(e) = resp.send(&mut writer).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_multiple_extra_header() {
        let mut resp = HttpResponse::<3>::new();
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-One", "Bar")) {
            self::panic!("{:?}", e);
        }
        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-Two", "Baz")) {
            self::panic!("{:?}", e);
        }
        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-Three", "Bat")) {
            self::panic!("{:?}", e);
        }

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
Foo-One: Bar\r
Foo-Two: Baz\r
Foo-Three: Bat\r
\r
"
        .as_bytes();

        if let Err(e) = resp.send(&mut writer).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_multiple_replaced_extra_header() {
        let mut resp = HttpResponse::<3>::new();
        let mut dst = Vec::<u8>::new();
        let mut writer = TestWriter::new(&mut dst);

        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-One", "Bar")) {
            self::panic!("{:?}", e);
        }
        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-Two", "Baz")) {
            self::panic!("{:?}", e);
        }
        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-Three", "Bat")) {
            self::panic!("{:?}", e);
        }

        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-Two", "Updated")) {
            self::panic!("{:?}", e);
        }
        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-Three", "Updated")) {
            self::panic!("{:?}", e);
        }

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
Foo-One: Bar\r
Foo-Two: Updated\r
Foo-Three: Updated\r
\r
"
        .as_bytes();

        if let Err(e) = resp.send(&mut writer).await {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_excess_headers_errors() {
        let mut resp = HttpResponse::<3>::new();

        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-One", "Bar")) {
            self::panic!("{:?}", e);
        }
        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-Two", "Baz")) {
            self::panic!("{:?}", e);
        }
        if let Err(e) = resp.add_extra_header(HttpHeader::Other("Foo-Three", "Bat")) {
            self::panic!("{:?}", e);
        }

        assert_eq!(
            resp.add_extra_header(HttpHeader::Other("Oops", "Too Many")),
            Err(HTTPError::ExtraHeadersExceeded)
        );
    }
}
