#![no_std]

use core::{fmt, u32};
use embedded_storage::{nor_flash::NorFlash, nor_flash::ReadNorFlash};
use serde::de::Visitor;
use serde::{Deserialize, Serialize};

const CONFIGV1_MAGIC: [u8; 13] = [
    b'd', b'o', b'o', b'r', b'c', b'o', b'n', b't', b'r', b'o', b'l', b'v', b'1',
];

#[derive(Clone, Copy, Debug)]
pub struct ConfigV1Value([u8; 64]);

impl ConfigV1Value {
    pub fn as_str(&self) -> &str {
        if let Some(null_offset) = self.0.iter().position(|e| *e == 0u8) {
            if null_offset == 0 {
                return "";
            }
            return str::from_utf8(&self.0[..null_offset]).unwrap_or("");
        }

        str::from_utf8(&self.0).unwrap_or("")
    }
}

impl TryFrom<&str> for ConfigV1Value {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut ret = ConfigV1Value::default();
        let data = value.as_bytes();
        if data.len() > ret.0.len() {
            return Err("input string too long (>64 bytes)");
        }

        ret.0[..data.len()].copy_from_slice(&data);

        Ok(ret)
    }
}

impl Serialize for ConfigV1Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(null_offset) = self.0.iter().position(|e| *e == 0u8) {
            if null_offset == 0 {
                return serializer.serialize_str("");
            }
            return serializer.serialize_str(str::from_utf8(&self.0[..null_offset]).unwrap_or(""));
        }

        serializer.serialize_str(str::from_utf8(&self.0[..]).unwrap_or(""))
    }
}

impl<'de> Deserialize<'de> for ConfigV1Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ConfigV1ValueVisitor;

        impl<'de> Visitor<'de> for ConfigV1ValueVisitor {
            type Value = ConfigV1Value;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("expecting utf-8 string of <= 64 bytes")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let bytes = v.as_bytes();
                if bytes.len() > 64 {
                    return Err(E::custom("value more than 64 bytes"));
                }

                let mut ret = ConfigV1Value([0u8; 64]);
                ret.0[..bytes.len()].copy_from_slice(bytes);
                Ok(ret)
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v.len() > 64 {
                    return Err(E::custom("value more than 64 bytes"));
                }

                let mut ret = ConfigV1Value([0u8; 64]);
                ret.0[..v.len()].copy_from_slice(v);
                Ok(ret)
            }
        }

        deserializer.deserialize_str(ConfigV1ValueVisitor)
    }
}

impl Default for ConfigV1Value {
    fn default() -> Self {
        Self([0u8; 64])
    }
}

#[derive(Clone, Copy, Serialize, Debug)]
pub struct ConfigV1 {
    #[serde(skip)]
    pre_magic: ConfigV1Value,
    pub device_name: ConfigV1Value,
    pub wifi_ssid: ConfigV1Value,
    #[serde(skip_serializing)]
    pub wifi_pass: ConfigV1Value,
    pub mqtt_host: ConfigV1Value,
    pub mqtt_user: ConfigV1Value,
    #[serde(skip_serializing)]
    pub mqtt_pass: ConfigV1Value,
    #[serde(skip)]
    pub post_magic: ConfigV1Value,
}

impl Default for ConfigV1 {
    fn default() -> Self {
        let mut magic = ConfigV1Value([0u8; 64]);
        magic.0[..CONFIGV1_MAGIC.len()].copy_from_slice(&CONFIGV1_MAGIC);

        Self {
            pre_magic: magic,
            device_name: ConfigV1Value::default(),
            wifi_ssid: ConfigV1Value::default(),
            wifi_pass: ConfigV1Value::default(),
            mqtt_host: ConfigV1Value::default(),
            mqtt_user: ConfigV1Value::default(),
            mqtt_pass: ConfigV1Value::default(),
            post_magic: magic,
        }
    }
}

impl ConfigV1 {
    pub fn update(&mut self, update: &ConfigV1Update) {
        if let Some(value) = update.device_name {
            if value.0[0] != 0 {
                self.device_name = value;
            }
        }
        if let Some(value) = update.wifi_ssid {
            if value.0[0] != 0 {
                self.wifi_ssid = value
            }
        }
        if let Some(value) = update.wifi_pass {
            if value.0[0] != 0 {
                self.wifi_pass = value;
            }
        }
        if let Some(value) = update.mqtt_host {
            if value.0[0] != 0 {
                self.mqtt_host = value;
            }
        }
        if let Some(value) = update.mqtt_user {
            if value.0[0] != 0 {
                self.mqtt_user = value;
            }
        }
        if let Some(value) = update.mqtt_pass {
            if value.0[0] != 0 {
                self.mqtt_pass = value;
            }
        }
    }

    pub fn load<S: ReadNorFlash>(src: &mut S) -> Result<Self, &'static str> {
        let mut read_buf = [0u8; size_of::<ConfigV1>()];
        if let Err(_) = src.read(0, &mut read_buf[..]) {
            return Err("error reading config from storage");
        }

        let mut config = ConfigV1::default();

        let mut offset = 0;
        config
            .pre_magic
            .0
            .copy_from_slice(&read_buf[offset..offset + 64]);
        offset += 64;
        config
            .device_name
            .0
            .copy_from_slice(&read_buf[offset..offset + 64]);
        offset += 64;
        config
            .wifi_ssid
            .0
            .copy_from_slice(&read_buf[offset..offset + 64]);
        offset += 64;
        config
            .wifi_pass
            .0
            .copy_from_slice(&read_buf[offset..offset + 64]);
        offset += 64;
        config
            .mqtt_host
            .0
            .copy_from_slice(&read_buf[offset..offset + 64]);
        offset += 64;
        config
            .mqtt_user
            .0
            .copy_from_slice(&read_buf[offset..offset + 64]);
        offset += 64;
        config
            .mqtt_pass
            .0
            .copy_from_slice(&read_buf[offset..offset + 64]);
        offset += 64;
        config
            .post_magic
            .0
            .copy_from_slice(&read_buf[offset..offset + 64]);

        if config.pre_magic.0[..CONFIGV1_MAGIC.len()] != CONFIGV1_MAGIC[..] {
            return Err("no config exists or config corrupt");
        }

        if config.post_magic.0[..CONFIGV1_MAGIC.len()] != CONFIGV1_MAGIC[..] {
            return Err("config corrupt");
        }

        Ok(config)
    }

    pub fn save<S: NorFlash>(&self, mut dst: S) -> Result<(), &'static str> {
        if !self.complete() {
            return Err("config not complete");
        }

        let mut write_buf = [0u8; size_of::<ConfigV1>()];
        let mut offset = 0;

        write_buf[offset..offset + 64].copy_from_slice(&self.pre_magic.0);
        offset += 64;

        write_buf[offset..offset + 64].copy_from_slice(&self.device_name.0);
        offset += 64;

        write_buf[offset..offset + 64].copy_from_slice(&self.wifi_ssid.0);
        offset += 64;

        write_buf[offset..offset + 64].copy_from_slice(&self.wifi_pass.0);
        offset += 64;

        write_buf[offset..offset + 64].copy_from_slice(&self.mqtt_host.0);
        offset += 64;

        write_buf[offset..offset + 64].copy_from_slice(&self.mqtt_user.0);
        offset += 64;

        write_buf[offset..offset + 64].copy_from_slice(&self.mqtt_pass.0);
        offset += 64;

        write_buf[offset..offset + 64].copy_from_slice(&self.post_magic.0);

        let erase_len: u32 = 4096;
        if let Err(_) = dst.erase(0, erase_len) {
            return Err("error erasing flash prior to write");
        }
        if let Err(_) = dst.write(0, &write_buf) {
            return Err("error writing to storage");
        }

        Ok(())
    }

    fn complete(&self) -> bool {
        if self.device_name.0[0] == 0u8 {
            return false;
        }
        if self.wifi_ssid.0[0] == 0u8 {
            return false;
        }
        if self.wifi_pass.0[0] == 0u8 {
            return false;
        }
        if self.mqtt_host.0[0] == 0u8 {
            return false;
        }
        if self.mqtt_pass.0[0] == 0u8 {
            return false;
        }

        true
    }
}

#[derive(Deserialize)]
pub struct ConfigV1Update {
    device_name: Option<ConfigV1Value>,
    wifi_ssid: Option<ConfigV1Value>,
    wifi_pass: Option<ConfigV1Value>,
    mqtt_host: Option<ConfigV1Value>,
    mqtt_user: Option<ConfigV1Value>,
    mqtt_pass: Option<ConfigV1Value>,
}

#[cfg(test)]
mod tests {
    extern crate std;
    use serde_json_core::{from_str, to_slice};

    use super::*;

    #[test]
    fn test_deserialize_update() {
        let json =
            "{\"device_name\": \"mydoor\", \"wifi_ssid\": \"mywifi\", \"wifi_pass\": \"mypass\"}";

        let config_update = from_str::<ConfigV1Update>(json);
        if let Err(e) = config_update {
            assert!(false, "deserializing update returned err: {}", e);
        }

        let config_update = config_update.unwrap().0;
        assert!(
            config_update.device_name.is_some(),
            "device_name should be Some"
        );
        assert!(
            config_update.wifi_ssid.is_some(),
            "wifi_ssid should be Some"
        );
        assert!(
            config_update.wifi_pass.is_some(),
            "wifi_pass should be Some"
        );
        assert!(
            config_update.mqtt_host.is_none(),
            "mqtt_host should be None"
        );
        assert!(
            config_update.mqtt_pass.is_none(),
            "mqtt_pass should be None"
        );

        let mut config = ConfigV1::default();
        config.update(&config_update);

        assert_eq!(
            config.device_name.as_str(),
            "mydoor",
            "device name should be 'mydoor'"
        );
    }

    #[test]
    fn test_serialize_config() {
        let mut config = ConfigV1::default();
        config.device_name = "mydevice".try_into().unwrap();

        let mut serialized = [0u8; 1024];

        match to_slice(&config, &mut serialized[..]) {
            Ok(n) => assert_eq!(str::from_utf8(&serialized[..n]).unwrap_or("not_utf8"), ""),
            Err(e) => assert!(false, "serialization returned error: {}", e),
        }
    }
}
