use embedded_io_async::{Error, ErrorKind, Read, Write};

use crate::http::HTTPError;
use crate::http::request::HttpRequest;
use crate::http::response::{HttpResponder, HttpResponderStateInit};
use crate::http::websocket::Websocket;

pub trait RequestHandler {
    fn handle_request<'client, 'buff, C: Read + Write + 'client>(
        &self,
        req: HttpRequest<'buff>,
        resp: HttpResponder<'buff, 'client, C, HttpResponderStateInit>,
    ) -> impl Future<Output = Result<Option<Websocket<'client, C>>, HTTPError>>;

    fn handle_websocket<'client, C: Read + Write + 'client>(
        &self,
        mut _websocket: Websocket<'client, C>,
        _buffer: &mut [u8],
    ) -> impl Future<Output = Result<(), HTTPError>> {
        async { Err(HTTPError::UnsupportedRequest("websocket not implemented")) }
    }
}

pub struct HTTPServer<H> {
    handler: H,
}

impl<H> HTTPServer<H>
where
    H: RequestHandler,
{
    pub fn new(handler: H) -> Self {
        Self { handler }
    }

    pub async fn serve<C>(&self, client: &mut C, http_buff: &mut [u8]) -> Result<(), HTTPError>
    where
        C: Read + Write,
    {
        'client: loop {
            let mut http_buff_offset = 0;
            loop {
                let res = client.read(&mut http_buff[http_buff_offset..]).await;
                match res {
                    Ok(0) => {
                        break 'client;
                    }
                    Ok(n) => {
                        http_buff_offset += n;
                        match HttpRequest::try_from(&http_buff[..]) {
                            Ok(request) => {
                                // handle request for response
                                let resp = HttpResponder::<'_, '_, _, HttpResponderStateInit>::new(
                                    &request, client,
                                );
                                if let Some(ws) = self.handler.handle_request(request, resp).await?
                                {
                                    return self.handler.handle_websocket(ws, http_buff).await;
                                }

                                break;
                            }
                            Err(HTTPError::Incomplete) => continue,
                            Err(e) => return Err(e),
                        };
                    }
                    Err(e) if e.kind() == ErrorKind::ConnectionReset => {
                        break 'client;
                    }
                    Err(_) => {
                        return Err(HTTPError::NetworkError("unexpected network error"));
                    }
                };
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;
    use std::*;

    use embedded_io_async::{ErrorKind, ErrorType};

    use super::*;
    use crate::http::response::HttpStatusCode;
    use crate::http::websocket::Websocket;

    struct TestReader<'a> {
        max_reads: usize,
        reads: usize,
        inner: &'a mut Vec<u8>,
    }

    impl<'a> TestReader<'a> {
        fn new(inner: &'a mut Vec<u8>, max_reads: usize) -> Self {
            Self {
                inner,
                max_reads,
                reads: 0,
            }
        }
    }

    impl<'a> ErrorType for TestReader<'a> {
        type Error = ErrorKind;
    }

    impl<'a> Read for TestReader<'a> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.reads >= self.max_reads {
                return Err(Self::Error::ConnectionReset);
            }
            self.reads += 1;

            if self.inner.len() > buf.len() {
                buf.copy_from_slice(&self.inner[..buf.len()]);
                return Ok(buf.len());
            }

            buf[..self.inner.len()].copy_from_slice(&self.inner[..]);
            Ok(self.inner.len())
        }
    }

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

    struct TestReaderWriter<'a> {
        reader: TestReader<'a>,
        writer: TestWriter<'a>,
    }

    impl<'a> ErrorType for TestReaderWriter<'a> {
        type Error = ErrorKind;
    }

    impl<'a> Read for TestReaderWriter<'a> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            self.reader.read(buf).await
        }
    }

    impl<'a> Write for TestReaderWriter<'a> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.writer.write(buf).await
        }

        async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
            self.writer.inner.extend_from_slice(buf);
            Ok(())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            self.writer.flush().await
        }
    }

    struct Handler {}
    impl RequestHandler for Handler {
        async fn handle_request<'buff, 'client, C: Read + Write + 'client>(
            &self,
            req: HttpRequest<'buff>,
            resp: HttpResponder<'buff, 'client, C, HttpResponderStateInit>,
        ) -> Result<Option<Websocket<'client, C>>, HTTPError> {
            match req.path {
                "/index.html" => {
                    resp.with_status(HttpStatusCode::OK)
                        .await?
                        .with_body("working".as_bytes())
                        .await?
                }
                "/test1" => {
                    resp.with_status(HttpStatusCode::OK)
                        .await?
                        .with_body("test1".as_bytes())
                        .await?
                }
                _ => {
                    resp.with_status(HttpStatusCode::NotFound)
                        .await?
                        .with_body("Not Found".as_bytes())
                        .await?
                }
            }
            Ok(None)
        }
    }

    #[tokio::test]
    async fn test_http_server() {
        let handler = Handler {};
        let server = HTTPServer::<Handler>::new(handler);

        let mut reader_buf = "GET /index.html HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc"
            .as_bytes()
            .to_vec();
        let mut writer_buf = Vec::<u8>::new();

        let mut client = TestReaderWriter {
            reader: TestReader::new(&mut reader_buf, 1),
            writer: TestWriter::new(&mut writer_buf),
        };

        let mut http_buff = [0u8; 2048];

        match server.serve(&mut client, &mut http_buff[..]).await {
            Ok(_) => {}
            Err(HTTPError::Disconnected) => {}
            Err(e) => {
                std::panic!("{:?}", e);
            }
        }

        assert_eq!(
            writer_buf.as_slice(),
            "HTTP/1.1 200 OK\r
Server: unspecified\r
Content-Length: 7\r
\r
working"
                .as_bytes()
        );
    }
}
