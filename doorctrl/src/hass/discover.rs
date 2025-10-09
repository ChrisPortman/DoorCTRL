use serde::Serialize;

const DEVICE_NAME: &str = env!("DEVICE_NAME");
const LOCK_ID: &str = env!("LOCK_ID");
const SENSOR_ID: &str = env!("SENSOR_ID");

const MQTT_PAYLOAD_AVAILABLE: &str = "online";
const MQTT_PAYLOAD_NOT_AVAILABLE: &str = "offline";
const MQTT_AVAILABILITY_MODE: &str = "latest";
const MQTT_PAYLOAD_LOCK: &str = "LOCK";
const MQTT_PAYLOAD_UNLOCK: &str = "UNLOCK";
const MQTT_STATE_LOCKED: &str = "LOCKED";
const MQTT_STATE_UNLOCKED: &str = "UNLOCKED";
const MQTT_STATE_OFF: &str = "OFF";
const MQTT_STATE_ON: &str = "ON";
const MQTT_PLATFORM_LOCK: &str = "lock";
const MQTT_PLATFORM_BINARY_SENSOR: &str = "binary_sensor";
const MQTT_DEVICE_CLASS_BINARY_SENSOR: &str = "door";

const MQTT_ORIGIN_NAME: &str = "doorctl";
const MQTT_ORIGIN_SW_VERSION: &str = "0.0.1";
const MQTT_ORIGIN_SUPPORT_URL: &str = "https://github.com/chrisportman/doorctl";

#[derive(Serialize)]
struct DiscoveryDevice {
    identifiers: &'static str,
    name: &'static str,
}

impl Default for DiscoveryDevice {
    fn default() -> Self {
        Self {
            identifiers: DEVICE_NAME,
            name: DEVICE_NAME,
        }
    }
}

#[derive(Serialize)]
struct DiscoveryOrigin {
    name: &'static str,
    sw_version: &'static str,
    support_url: &'static str,
}

impl Default for DiscoveryOrigin {
    fn default() -> Self {
        Self {
            name: MQTT_ORIGIN_NAME,
            sw_version: MQTT_ORIGIN_SW_VERSION,
            support_url: MQTT_ORIGIN_SUPPORT_URL,
        }
    }
}

#[derive(Serialize)]
struct ComponentLock<'a> {
    unique_id: &'static str,
    platform: &'static str,
    name: &'static str,
    enabled_by_default: bool,
    state_topic: &'a str,
    command_topic: &'a str,
    payload_lock: &'static str,
    payload_unlock: &'static str,
    state_locked: &'static str,
    state_unlocked: &'static str,
    optimistic: bool,
    retain: bool,
}

impl<'a> Default for ComponentLock<'a> {
    fn default() -> Self {
        Self {
            unique_id: LOCK_ID,
            platform: MQTT_PLATFORM_LOCK,
            name: "Lock",
            enabled_by_default: true,
            state_topic: "",
            command_topic: "",
            payload_lock: MQTT_PAYLOAD_LOCK,
            payload_unlock: MQTT_PAYLOAD_UNLOCK,
            state_locked: MQTT_STATE_LOCKED,
            state_unlocked: MQTT_STATE_UNLOCKED,
            optimistic: false,
            retain: false,
        }
    }
}

#[derive(Serialize)]
struct ComponentBinarySensor<'a> {
    unique_id: &'static str,
    device_class: &'static str,
    name: &'static str,
    platform: &'static str,
    enabled_by_default: bool,
    state_topic: &'a str,
    payload_on: &'static str,
    payload_off: &'static str,
    optimistic: bool,
    retain: bool,
}

impl<'a> Default for ComponentBinarySensor<'a> {
    fn default() -> Self {
        Self {
            unique_id: SENSOR_ID,
            device_class: MQTT_DEVICE_CLASS_BINARY_SENSOR,
            name: "Door",
            platform: MQTT_PLATFORM_BINARY_SENSOR,
            enabled_by_default: true,
            state_topic: "",
            payload_on: MQTT_STATE_ON,
            payload_off: MQTT_STATE_OFF,
            optimistic: false,
            retain: false,
        }
    }
}

#[derive(Serialize, Default)]
struct DiscoveryComponents<'a> {
    lock: ComponentLock<'a>,
    reed: ComponentBinarySensor<'a>,
}

#[derive(Serialize, Default)]
pub(crate) struct Discovery<'a> {
    device: DiscoveryDevice,
    origin: DiscoveryOrigin,
    components: DiscoveryComponents<'a>,
    availability_topic: &'a str,
    availability_mode: &'static str,
    qos: u8,
}

impl<'a> Discovery<'a> {
    pub(crate) fn new(
        avail_topic: &'a str,
        lock_state_topic: &'a str,
        lock_cmd_topic: &'a str,
        reed_state_topic: &'a str,
    ) -> Self {
        let mut disc = Discovery::default();
        disc.availability_topic = avail_topic;
        disc.availability_mode = MQTT_AVAILABILITY_MODE;
        disc.components.lock.state_topic = lock_state_topic;
        disc.components.lock.command_topic = lock_cmd_topic;
        disc.components.reed.state_topic = reed_state_topic;
        disc
    }
}
