use base64ct::{Base64, Encoding};
use embedded_io_async::Write;
use sha1::{Digest, Sha1};

use crate::{AsciiInt, CR, LF};

const RESPONSE_101: &'static str = "HTTP/1.1 101 Switching Protocols\r
Upgrade: websocket\r
Connection: Upgrade\r
Sec-WebSocket-Accept: ";

const RESPONSE_400: &'static str = "HTTP/1.1 400 Bad Request\r
Server: DoorCtrl\r
Content-Type: text/html\r
Content-Length: ";

const RESPONSE_400_BODY: &'static str = "
<!DOCTYPE html>
<html>
<head>
<title>DoorCTRL</title>
</head>
<body>
<p>400 Request not supported</p>
</body>
</html>
";

const RESPONSE_404: &'static str = "HTTP/1.1 404 Not Found\r
Server: DoorCtrl\r
Content-Type: text/html\r
Content-Length: ";

const RESPONSE_404_BODY: &'static str = "
<!DOCTYPE html>
<html>
<head>
<title>DoorCTRL</title>
</head>
<body>
<p>404 Not Found</p>
</body>
</html>
";

const RESPONSE_200: &'static str = "HTTP/1.1 200 OK\r
Server: DoorCtrl\r
Content-Type: text/html\r
Content-Length: ";

pub const RESPONSE_200_BODY: &'static str = "
<!DOCTYPE html>
<html>
<head>
<title>DoorCTRL</title>
</head>
<body>

<h1>DoorCTL is Alive</h1>
<p>Apparently the web server works...</p>

<script>
const ws = new WebSocket('/ws');

ws.addEventListener('open', (e) => {
  console.log('websocket opened');
  console.log(e);
});

ws.addEventListener('error', (e) => {
  console.log('websocket error');
  console.log(e);
});

ws.addEventListener('close', (e) => {
  console.log('websocket closed');
  console.log(e);
});

ws.addEventListener('message', (e) => {
  console.log('websocket message received');
  console.log(e);
});
</script>
</body>
</html>
";

const HTTP_WRITE_ERR: &'static str = "error writing http response to destination";

pub async fn respond<T: Write>(code: u16, dest: &mut T) -> Result<(), &'static str> {
    let headers: &[u8];
    let body: &[u8];

    match code {
        200 => {
            headers = RESPONSE_200.as_bytes();
            body = RESPONSE_200_BODY.as_bytes();
        }
        400 => {
            headers = RESPONSE_400.as_bytes();
            body = RESPONSE_400_BODY.as_bytes();
        }
        404 => {
            headers = RESPONSE_404.as_bytes();
            body = RESPONSE_404_BODY.as_bytes();
        }
        _ => {
            return Err("unsupported http code");
        }
    };

    let content_len: AsciiInt = (body.len() as u64).into();

    if let Err(_) = dest.write(headers).await {
        return Err(HTTP_WRITE_ERR);
    }
    if let Err(_) = dest.write(&content_len.as_bytes()).await {
        return Err(HTTP_WRITE_ERR);
    }
    if let Err(_) = dest.write(&[CR, LF, CR, LF]).await {
        return Err(HTTP_WRITE_ERR);
    }
    if let Err(_) = dest.write(body).await {
        return Err(HTTP_WRITE_ERR);
    }

    Ok(())
}

const SEC_WEBSOCKET_ACCEPT_MAGIC: &'static str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub async fn respond_websocket<T: Write>(key: &str, dest: &mut T) -> Result<(), &'static str> {
    let mut key_hasher = Sha1::new();
    key_hasher.update(key.as_bytes());
    key_hasher.update(SEC_WEBSOCKET_ACCEPT_MAGIC.as_bytes());
    let key_hash = key_hasher.finalize();

    let mut key_b64_buff = [0u8; 28];
    let encoded = match Base64::encode(&key_hash, &mut key_b64_buff) {
        Ok(e) => e,
        Err(_) => {
            return Err("error enoding key hash");
        }
    };

    if let Err(_) = dest.write(RESPONSE_101.as_bytes()).await {
        return Err(HTTP_WRITE_ERR);
    }

    if let Err(_) = dest.write(&encoded.as_bytes()).await {
        return Err(HTTP_WRITE_ERR);
    }

    if let Err(_) = dest.write(&[CR, LF, CR, LF]).await {
        return Err(HTTP_WRITE_ERR);
    }

    Ok(())
}
