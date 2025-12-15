use core::marker::PhantomData;
use embedded_io_async::{Read, Write};

use crate::http::ascii::{AsciiInt, CR, LF, SP};
use crate::http::header::{RequestHeader, ResponseHeader};
use crate::http::request::HttpRequest;
use crate::http::websocket::{Websocket, sec_websocket_accept_val};
use crate::http::{HTTPError, HttpWrite};

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
                if !(100..=599).contains(&n){
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
        ))).or(Err(HTTPError::Disconnected))
    }
}

pub struct HttpResponderStateInit;
pub struct HttpResponderStateSending;

pub struct HttpResponder<'a, 'client, C: Read + Write, State> {
    status: HttpStatusCode,
    server: ResponseHeader<'a>,
    client: &'client mut C,
    finished: bool,
    _state: PhantomData<State>,
}

impl<'a, 'client, C: Read + Write> HttpResponder<'a, 'client, C, HttpResponderStateInit> {
    pub fn new(request: &HttpRequest<'a>, client: &'client mut C) -> Self {
        Self {
            client,
            status: HttpStatusCode::OK,
            server: ResponseHeader::Server(request.host),
            finished: false,
            _state: PhantomData,
        }
    }

    #[must_use = "http responder not finished with either `with_body` or `no_body` results in a client waiting for data"]
    pub async fn with_status(
        self,
        status: HttpStatusCode,
    ) -> Result<HttpResponder<'a, 'client, C, HttpResponderStateSending>, HTTPError> {
        status.write(self.client).await?;
        self.server.write(self.client).await?;

        Ok(HttpResponder::<'a, 'client, C, HttpResponderStateSending> {
            status,
            server: self.server,
            client: self.client,
            finished: self.finished,
            _state: PhantomData,
        })
    }

    #[must_use = "http responder not finished with either `with_body` or `no_body` results in a client waiting for data"]
    pub async fn with_header(
        self,
        header: ResponseHeader<'a>,
    ) -> Result<HttpResponder<'a, 'client, C, HttpResponderStateSending>, HTTPError> {
        let status = self.status;

        self.with_status(status).await?.with_header(header).await
    }

    pub async fn upgrade(self, req: HttpRequest<'a>) -> Result<Websocket<'client, C>, HTTPError> {
        let websocket_key = match req.get_header(RequestHeader::SecWebSocketKey("")) {
            Some(RequestHeader::SecWebSocketKey(k)) => k,
            _ => {
                self.with_status(HttpStatusCode::BadRequest)
                    .await?
                    .no_body()
                    .await?;
                return Err(HTTPError::ProtocolError(
                    "websocket upgrade did not include a Sec-Websocket-Key header",
                ));
            }
        };

        let accept_key = match sec_websocket_accept_val(websocket_key) {
            Ok(k) => k,
            Err(e) => {
                self.with_status(HttpStatusCode::BadRequest)
                    .await?
                    .no_body()
                    .await?;
                return Err(HTTPError::ProtocolError(e));
            }
        };

        return self
            .with_status(HttpStatusCode::SwitchingProtocols)
            .await?
            .with_header(ResponseHeader::SecWebSocketAccept(accept_key))
            .await?
            .with_header(ResponseHeader::Other("Upgrade", "websocket"))
            .await?
            .with_header(ResponseHeader::Connection("Upgrade"))
            .await?
            .websocket()
            .await;
    }
}

impl<'a, 'client, C: Read + Write> HttpResponder<'a, 'client, C, HttpResponderStateSending> {
    #[must_use = "http responder not finished with either `with_body` or `no_body` results in a client waiting for data"]
    pub async fn with_header(self, header: ResponseHeader<'a>) -> Result<Self, HTTPError> {
        header.write(self.client).await?;

        Ok(self)
    }

    pub async fn no_body(self) -> Result<(), HTTPError> {
        self.client
            .write_all(&[CR, LF])
            .await
            .or(Err(HTTPError::Disconnected))?;

        Ok(())
    }

    pub async fn with_body(self, body: &[u8]) -> Result<(), HTTPError> {
        ResponseHeader::ContentLength(body.len())
            .write(self.client)
            .await?;

        self.client
            .write_all(&[CR, LF])
            .await
            .or(Err(HTTPError::NetworkError("connection reset by peer")))?;

        if self.client.write_all(body).await.is_err() {
            return Err(HTTPError::Disconnected);
        }

        Ok(())
    }

    async fn websocket(self) -> Result<Websocket<'client, C>, HTTPError> {
        self.client
            .write_all(&[CR, LF])
            .await
            .or(Err(HTTPError::Disconnected))?;

        Ok(Websocket::new(self.client))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use embedded_io_async::{ErrorKind, ErrorType};
    use std::vec::Vec;
    use std::*;

    use crate::http::request::HttpMethod;

    use super::*;

    struct TestClient<'a> {
        inner: &'a mut Vec<u8>,
    }

    impl<'a> TestClient<'a> {
        fn new(inner: &'a mut Vec<u8>) -> Self {
            Self { inner: inner }
        }
    }

    impl<'a> ErrorType for TestClient<'a> {
        type Error = ErrorKind;
    }

    impl<'a> Write for TestClient<'a> {
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

    impl<'a> Read for TestClient<'a> {
        async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, Self::Error> {
            Ok(0)
        }
    }

    // HTTP uses `\r\n` as EOL delimeters.  In the expected data, we manually add
    // the \r at the end of the line, before the inherrent \n.

    #[tokio::test]
    async fn test_http_response_default() {
        let request = HttpRequest::<'_> {
            method: HttpMethod::GET,
            path: "/",
            host: "RustServer",
            content_type: None,
            user_agent: None,
            content_length: 0,
            body: None,
            header_slice: None,
        };

        let mut dst = Vec::<u8>::new();
        let mut writer = TestClient::new(&mut dst);
        // let resp = HttpResponse::<3>::new();
        let resp =
            HttpResponder::<'_, '_, TestClient, HttpResponderStateInit>::new(&request, &mut writer);

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
\r
"
        .as_bytes();

        resp.with_status(HttpStatusCode::OK)
            .await
            .unwrap()
            .with_header(ResponseHeader::ContentType("text/html"))
            .await
            .unwrap()
            .no_body()
            .await
            .unwrap();

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_default_with_body() {
        let request = HttpRequest::<'_> {
            method: HttpMethod::GET,
            path: "/",
            host: "RustServer",
            content_type: None,
            user_agent: None,
            content_length: 0,
            body: None,
            header_slice: None,
        };

        let mut dst = Vec::<u8>::new();
        let mut writer = TestClient::new(&mut dst);
        // let resp = HttpResponse::<3>::new();
        let resp =
            HttpResponder::<'_, '_, TestClient, HttpResponderStateInit>::new(&request, &mut writer);

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

        resp.with_status(HttpStatusCode::OK)
            .await
            .unwrap()
            .with_header(ResponseHeader::ContentType("text/html"))
            .await
            .unwrap()
            .with_body(body)
            .await
            .unwrap();

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
Content-Length: 114\r
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

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_status() {
        let request = HttpRequest::<'_> {
            method: HttpMethod::GET,
            path: "/",
            host: "RustServer",
            content_type: None,
            user_agent: None,
            content_length: 0,
            body: None,
            header_slice: None,
        };

        let mut dst = Vec::<u8>::new();
        let mut writer = TestClient::new(&mut dst);
        let resp =
            HttpResponder::<'_, '_, TestClient, HttpResponderStateInit>::new(&request, &mut writer);

        resp.with_status(HttpStatusCode::NotFound)
            .await
            .unwrap()
            .with_header(ResponseHeader::ContentType("text/html"))
            .await
            .unwrap()
            .no_body()
            .await
            .unwrap();

        let expected = "HTTP/1.1 404 Not Found\r
Server: RustServer\r
Content-Type: text/html\r
\r
"
        .as_bytes();

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_custom_status() {
        let request = HttpRequest::<'_> {
            method: HttpMethod::GET,
            path: "/",
            host: "RustServer",
            content_type: None,
            user_agent: None,
            content_length: 0,
            body: None,
            header_slice: None,
        };

        let mut dst = Vec::<u8>::new();
        let mut writer = TestClient::new(&mut dst);
        let resp =
            HttpResponder::<'_, '_, TestClient, HttpResponderStateInit>::new(&request, &mut writer);

        resp.with_status(HttpStatusCode::Other(401))
            .await
            .unwrap()
            .with_header(ResponseHeader::ContentType("text/html"))
            .await
            .unwrap()
            .no_body()
            .await
            .unwrap();

        let expected = "HTTP/1.1 401\r
Server: RustServer\r
Content-Type: text/html\r
\r
"
        .as_bytes();

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_custom_content_type() {
        let request = HttpRequest::<'_> {
            method: HttpMethod::GET,
            path: "/",
            host: "RustServer",
            content_type: None,
            user_agent: None,
            content_length: 0,
            body: None,
            header_slice: None,
        };

        let mut dst = Vec::<u8>::new();
        let mut writer = TestClient::new(&mut dst);
        let resp =
            HttpResponder::<'_, '_, TestClient, HttpResponderStateInit>::new(&request, &mut writer);

        resp.with_status(HttpStatusCode::OK)
            .await
            .unwrap()
            .with_header(ResponseHeader::ContentType("application/json"))
            .await
            .unwrap()
            .no_body()
            .await
            .unwrap();

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: application/json\r
\r
"
        .as_bytes();

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_custom_server() {
        let request = HttpRequest::<'_> {
            method: HttpMethod::GET,
            path: "/",
            host: "FancyServer",
            content_type: None,
            user_agent: None,
            content_length: 0,
            body: None,
            header_slice: None,
        };

        let mut dst = Vec::<u8>::new();
        let mut writer = TestClient::new(&mut dst);
        let resp =
            HttpResponder::<'_, '_, TestClient, HttpResponderStateInit>::new(&request, &mut writer);

        resp.with_status(HttpStatusCode::OK)
            .await
            .unwrap()
            .with_header(ResponseHeader::ContentType("text/html"))
            .await
            .unwrap()
            .no_body()
            .await
            .unwrap();

        let expected = "HTTP/1.1 200 OK\r
Server: FancyServer\r
Content-Type: text/html\r
\r
"
        .as_bytes();

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_one_extra_header() {
        let request = HttpRequest::<'_> {
            method: HttpMethod::GET,
            path: "/",
            host: "RustServer",
            content_type: None,
            user_agent: None,
            content_length: 0,
            body: None,
            header_slice: None,
        };

        let mut dst = Vec::<u8>::new();
        let mut writer = TestClient::new(&mut dst);
        let resp =
            HttpResponder::<'_, '_, TestClient, HttpResponderStateInit>::new(&request, &mut writer);

        resp.with_status(HttpStatusCode::OK)
            .await
            .unwrap()
            .with_header(ResponseHeader::ContentType("text/html"))
            .await
            .unwrap()
            .with_header(ResponseHeader::Other("Foo", "Bar"))
            .await
            .unwrap()
            .no_body()
            .await
            .unwrap();

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
Foo: Bar\r
\r
"
        .as_bytes();

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }

    #[tokio::test]
    async fn test_http_response_with_multiple_extra_header() {
        let request = HttpRequest::<'_> {
            method: HttpMethod::GET,
            path: "/",
            host: "RustServer",
            content_type: None,
            user_agent: None,
            content_length: 0,
            body: None,
            header_slice: None,
        };

        let mut dst = Vec::<u8>::new();
        let mut writer = TestClient::new(&mut dst);
        let resp =
            HttpResponder::<'_, '_, TestClient, HttpResponderStateInit>::new(&request, &mut writer);

        resp.with_status(HttpStatusCode::OK)
            .await
            .unwrap()
            .with_header(ResponseHeader::ContentType("text/html"))
            .await
            .unwrap()
            .with_header(ResponseHeader::Other("Foo-One", "Bar"))
            .await
            .unwrap()
            .with_header(ResponseHeader::Other("Foo-Two", "Baz"))
            .await
            .unwrap()
            .with_header(ResponseHeader::Other("Foo-Three", "Bat"))
            .await
            .unwrap()
            .no_body()
            .await
            .unwrap();

        let expected = "HTTP/1.1 200 OK\r
Server: RustServer\r
Content-Type: text/html\r
Foo-One: Bar\r
Foo-Two: Baz\r
Foo-Three: Bat\r
\r
"
        .as_bytes();

        assert_eq!(
            &dst,
            expected,
            "oops, got:\n{}",
            str::from_utf8(&dst).unwrap()
        );
    }
}
