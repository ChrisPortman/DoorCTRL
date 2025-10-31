use core::fmt;
use embedded_storage::{ReadStorage, Storage};
use serde::de::Visitor;
use serde::{Deserialize, Serialize};

const CONFIGV1_MAGIC: [u8; 13] = [
    b'd', b'o', b'o', b'r', b'c', b'o', b'n', b't', b'r', b'o', b'l', b'v', b'1',
];

#[derive(Clone, Copy)]
struct ConfigV1Value([u8; 64]);

impl Serialize for ConfigV1Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(null_offset) = self.0.iter().position(|e| *e == 0u8) {
            return serializer.serialize_bytes(&self.0[..null_offset]);
        }

        serializer.serialize_bytes(&self.0[..])
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

#[derive(Clone, Copy, Serialize)]
pub struct ConfigV1 {
    #[serde(skip)]
    pre_magic: ConfigV1Value,
    device_name: ConfigV1Value,
    wifi_ssid: ConfigV1Value,
    #[serde(skip_serializing)]
    wifi_pass: ConfigV1Value,
    mqtt_host: ConfigV1Value,
    #[serde(skip_serializing)]
    mqtt_pass: ConfigV1Value,
    #[serde(skip)]
    post_magic: ConfigV1Value,
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
            mqtt_pass: ConfigV1Value::default(),
            post_magic: magic,
        }
    }
}

impl ConfigV1 {
    pub fn update(&mut self, update: &ConfigV1Update) {
        if let Some(value) = update.device_name {
            self.device_name = value;
        }
        if let Some(value) = update.wifi_ssid {
            self.wifi_ssid = value
        }
        if let Some(value) = update.wifi_pass {
            self.wifi_pass = value;
        }
        if let Some(value) = update.mqtt_host {
            self.mqtt_host = value;
        }
        if let Some(value) = update.mqtt_pass {
            self.mqtt_pass = value;
        }
    }

    pub fn load<S: ReadStorage>(mut src: S) -> Result<Self, &'static str> {
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

    pub fn save<S: Storage>(&self, mut dst: S) -> Result<(), &'static str> {
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

        write_buf[offset..offset + 64].copy_from_slice(&self.mqtt_pass.0);
        offset += 64;

        write_buf[offset..offset + 64].copy_from_slice(&self.post_magic.0);

        if let Err(_) = dst.write(0, &write_buf) {
            return Err("error writing to storage");
        }

        Ok(())
    }
}

#[derive(Deserialize)]
pub struct ConfigV1Update {
    device_name: Option<ConfigV1Value>,
    wifi_ssid: Option<ConfigV1Value>,
    wifi_pass: Option<ConfigV1Value>,
    mqtt_host: Option<ConfigV1Value>,
    mqtt_pass: Option<ConfigV1Value>,
}

#[cfg(test)]
mod tests {
    extern crate std;

    use esp_hal::config;
    use serde_json_core::from_str;

    use super::*;

    #[test]
    fn test_deserialise_update() {
        let json =
            "{\"device_name\": \"mydoor\", \"wifi_ssid\": \"mywifi\", \"wifi_pass\": \"mypass\"}";

        let configUpdate = from_str::<ConfigV1Update>(json);
        if let Err(e) = configUpdate {
            assert!(false, "deserializing update returned err: {}", e);
        }

        let configUpdate = configUpdate.unwrap().0;
        assert!(
            configUpdate.device_name.is_some(),
            "device_name should be Some"
        );
        assert!(configUpdate.wifi_ssid.is_some(), "wifi_ssid should be Some");
        assert!(configUpdate.wifi_pass.is_some(), "wifi_pass should be Some");
        assert!(configUpdate.mqtt_host.is_none(), "mqtt_host should be Some");
        assert!(configUpdate.mqtt_pass.is_none(), "mqtt_pass should be Some");
    }
}
