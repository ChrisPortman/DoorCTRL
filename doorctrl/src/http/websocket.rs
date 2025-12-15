use base64ct::{Base64, Encoding};
use embedded_io_async::{Read, Write};
use sha1::{Digest, Sha1};

const SEC_WEBSOCKET_ACCEPT_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub fn sec_websocket_accept_val(key: &str) -> Result<[u8; 28], &'static str> {
    let mut key_hasher = Sha1::new();
    key_hasher.update(key.as_bytes());
    key_hasher.update(SEC_WEBSOCKET_ACCEPT_MAGIC.as_bytes());
    let key_hash = key_hasher.finalize();

    let mut key_b64_buff = [0u8; 28];
    if Base64::encode(&key_hash, &mut key_b64_buff).is_err() {
        return Err("error enoding key hash due to invalid length");
    }

    Ok(key_b64_buff)
}

#[derive(defmt::Format, Debug)]
pub enum WebsocketError {
    InsufficientData(usize),
    Unsupported(&'static str),
    NetworkError,
}

// Basic receive process..
// 1. read 6 bytes from socket.  This is the minimum required for a Websocket frame header and
//    attemtp to decode, if there is an extended payload, a WebsocketError::InsufficientData(n)
//    will be returned where `n` indicates the mumber of additional bytes required to construct the
//    header. decode_header again with the additional bytes to recieve a header struct
// 2. read header.len bytes from the socket to receve the payload.

pub struct Websocket<'a, C: Read + Write> {
    conn: &'a mut C,
}

impl<'a, C: Read + Write> Websocket<'a, C> {
    pub fn new(conn: &'a mut C) -> Self {
        Self { conn }
    }

    pub async fn receive(&mut self, buf: &mut [u8]) -> Result<WebsocketFrame, WebsocketError> {
        let mut offset = 0;
        let mut header_buf = [0u8; 14];

        self.conn
            .read_exact(&mut header_buf[..6])
            .await
            .map_err(|_| WebsocketError::NetworkError)?;
        offset += 6;

        let header: WebsocketFrame;
        loop {
            header = match WebsocketFrame::decode(&header_buf[..offset]) {
                Ok(h) => h,
                Err(WebsocketError::InsufficientData(n)) => {
                    self.conn
                        .read_exact(&mut header_buf[offset..offset + n])
                        .await
                        .map_err(|_| WebsocketError::NetworkError)?;
                    offset += n;
                    continue;
                }
                Err(e) => {
                    return Err(e);
                }
            };
            break;
        }

        if header.len > buf.len() {
            return Err(WebsocketError::Unsupported(
                "payload length exceeds buffer size",
            ));
        }

        self.conn
            .read_exact(&mut buf[..header.len])
            .await
            .map_err(|_| WebsocketError::NetworkError)?;

        if header.masked {
            header.apply_mask(&mut buf[..header.len]);
        }

        Ok(header)
    }

    pub async fn send(&mut self, data: &mut [u8]) -> Result<(), WebsocketError> {
        let header = WebsocketFrame {
            fin: true,
            opcode: 2,
            masked: false,
            len: data.len(),
            mask: None,
        };

        let mut encoded_header = [0u8; 14];
        let header_len = header.encode(&mut encoded_header)?;

        self.conn
            .write_all(&encoded_header[..header_len])
            .await
            .map_err(|_| WebsocketError::NetworkError)?;

        self.conn
            .write_all(data)
            .await
            .map_err(|_| WebsocketError::NetworkError)?;

        Ok(())
    }
}

#[derive(defmt::Format, Debug)]
pub struct WebsocketFrame {
    pub opcode: u8,
    pub len: usize,
    fin: bool,
    masked: bool,
    mask: Option<[u8; 4]>,
}

impl WebsocketFrame {
    pub fn decode(value: &[u8]) -> Result<Self, WebsocketError> {
        let mut required_bytes = 2usize;

        if value.len() < required_bytes {
            return Err(WebsocketError::InsufficientData(
                required_bytes - value.len(),
            ));
        }

        let fin: bool = (value[0] & 128) == 128;
        let opcode: u8 = value[0] & 0x0F;

        if !fin || opcode == 0 {
            return Err(WebsocketError::Unsupported(
                "payload fragmentation not supported",
            ));
        }

        let masked: bool = (value[1] & 128) == 128;

        let mut len: u64 = (value[1] << 1 >> 1) as u64;
        let mut mask_offset = 2;
        if len == 126 {
            // 16 bit length field
            required_bytes += 2;
            if value.len() < required_bytes {
                return Err(WebsocketError::InsufficientData(
                    required_bytes - value.len(),
                ));
            }
            len = (value[2] as u64) << 8 | value[3] as u64;
            mask_offset = 4;
        }
        if len == 127 {
            // 64bit length field
            required_bytes += 8;
            if value.len() < required_bytes {
                return Err(WebsocketError::InsufficientData(
                    required_bytes - value.len(),
                ));
            }
            len = (value[2] as u64) << 56
                | (value[3] as u64) << 48
                | (value[4] as u64) << 40
                | (value[5] as u64) << 32
                | (value[6] as u64) << 24
                | (value[7] as u64) << 16
                | (value[8] as u64) << 8
                | value[9] as u64;
            mask_offset = 10;
        }

        let len: usize = match usize::try_from(len) {
            Ok(l) => l,
            Err(_) => {
                return Err(WebsocketError::Unsupported(
                    "payload length exceeds max platform architecture usize",
                ));
            }
        };

        let mut mask: Option<[u8; 4]> = None;

        if masked {
            required_bytes += 4;
            if value.len() < required_bytes {
                return Err(WebsocketError::InsufficientData(
                    required_bytes - value.len(),
                ));
            }

            mask = Some(value[mask_offset..mask_offset + 4].try_into().unwrap());
        }

        Ok(WebsocketFrame {
            fin,
            opcode,
            masked,
            len,
            mask,
        })
    }

    pub fn encode(&self, dest: &mut [u8]) -> Result<usize, WebsocketError> {
        if dest.len() < 2 {
            return Err(WebsocketError::Unsupported(
                "encode buffer requires at least 6 bytes",
            ));
        }

        // fin 1 MSB byte 1
        if self.fin {
            dest[0] ^= 0b1000_0000;
        }

        // opcode 4 LSB bits byte 1
        dest[0] ^= self.opcode & 0b000_1111;

        // masked 1 MSB byte 2
        if self.masked {
            dest[1] ^= 0b1000_0000;
        }

        let mut mask_offset = 2;
        if self.len <= 125 {
            if dest.len() < 2 {
                return Err(WebsocketError::Unsupported(
                    "encode buffer requires at least 6 bytes for given payload lenght",
                ));
            }
            // 7 LSB byte 2
            dest[1] ^= self.len as u8;
        }

        if self.len > 125 && self.len <= u16::MAX.into() {
            if dest.len() < 4 {
                return Err(WebsocketError::Unsupported(
                    "encode buffer requires at least 8 bytes for given payload lenght",
                ));
            }

            // indicate 16 bit length with byte 2 7LSB bits = 126
            dest[1] ^= 126u8;
            [dest[2], dest[3]] = (self.len as u16).to_be_bytes();
            mask_offset = 4;
        }

        if self.len > u16::MAX.into() {
            if dest.len() < 10 {
                return Err(WebsocketError::Unsupported(
                    "encode buffer requires at least 14 bytes for given payload lenght",
                ));
            }

            // indicate 64 bit length with byte 2 7LSB bits = 127
            dest[1] ^= 127u8;
            [
                dest[2], dest[3], dest[4], dest[5], dest[6], dest[7], dest[8], dest[9],
            ] = (self.len as u64).to_be_bytes();
            mask_offset = 10;
        }

        if let Some(mask) = self.mask {
            dest[mask_offset] ^= mask[0];
            dest[mask_offset + 1] ^= mask[1];
            dest[mask_offset + 2] ^= mask[2];
            dest[mask_offset + 3] ^= mask[3];

            return Ok(mask_offset + 4);
        }

        Ok(mask_offset)
    }

    fn apply_mask(&self, data: &mut [u8]) {
        if let Some(mask) = self.mask {
            for i in 0..self.len {
                data[i] ^= mask[i % 4];
            }
        }
    }
}
