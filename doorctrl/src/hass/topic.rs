const TOPIC_PREFIX: &str = "doorctl/";
const MQTT_TOPIC_SUFFIX_AVAILABILITY: &str = "/avail";
const MQTT_TOPIC_SUFFIX_LOCK_COMMAND: &str = "/lock/cmd/";
const MQTT_TOPIC_SUFFIX_LOCK_STATE: &str = "/lock/state";
const MQTT_TOPIC_SUFFIX_SENSOR_STATE: &str = "/reed/state";
const MQTT_TOPIC_DISCOVERY_PREFIX: &str = "homeassistant/device/";
const MQTT_TOPIC_DISCOVERY_SUFFIX: &str = "/config";

pub const MQTT_TOPIC_SENSOR_STATE_LEN: usize =
    TOPIC_PREFIX.len() + 12 + MQTT_TOPIC_SUFFIX_SENSOR_STATE.len();
pub const MQTT_TOPIC_LOCK_STATE_LEN: usize =
    TOPIC_PREFIX.len() + 12 + MQTT_TOPIC_SUFFIX_LOCK_STATE.len();
pub const MQTT_TOPIC_AVAILABILITY_LEN: usize =
    TOPIC_PREFIX.len() + 12 + MQTT_TOPIC_SUFFIX_AVAILABILITY.len();
pub const MQTT_TOPIC_LOCK_COMMAND_LEN: usize =
    TOPIC_PREFIX.len() + 12 + MQTT_TOPIC_SUFFIX_LOCK_COMMAND.len();
pub const MQTT_TOPIC_DISCOVERY_LEN: usize =
    MQTT_TOPIC_DISCOVERY_PREFIX.len() + 12 + MQTT_TOPIC_DISCOVERY_SUFFIX.len();

pub(super) fn mk_availability_topic(device_id: &[u8; 12]) -> [u8; MQTT_TOPIC_AVAILABILITY_LEN] {
    const SUFFIX: &str = MQTT_TOPIC_SUFFIX_AVAILABILITY;

    let mut topic = [0u8; MQTT_TOPIC_AVAILABILITY_LEN];

    let prefix_offset: usize = 0;
    let device_id_offset: usize = TOPIC_PREFIX.len();
    let suffix_offset: usize = device_id_offset + device_id.len();

    topic[prefix_offset..device_id_offset].copy_from_slice(TOPIC_PREFIX.as_bytes());
    topic[device_id_offset..suffix_offset].copy_from_slice(device_id);
    topic[suffix_offset..].copy_from_slice(SUFFIX.as_bytes());
    topic
}

pub(super) fn mk_lock_cmd_topic(device_id: &[u8; 12]) -> [u8; MQTT_TOPIC_LOCK_COMMAND_LEN] {
    const SUFFIX: &str = MQTT_TOPIC_SUFFIX_LOCK_COMMAND;

    let mut topic = [0u8; MQTT_TOPIC_LOCK_COMMAND_LEN];

    let prefix_offset: usize = 0;
    let device_id_offset: usize = TOPIC_PREFIX.len();
    let suffix_offset: usize = device_id_offset + device_id.len();

    topic[prefix_offset..device_id_offset].copy_from_slice(TOPIC_PREFIX.as_bytes());
    topic[device_id_offset..suffix_offset].copy_from_slice(device_id);
    topic[suffix_offset..].copy_from_slice(SUFFIX.as_bytes());
    topic
}

pub(super) fn mk_lock_state_topic(device_id: &[u8; 12]) -> [u8; MQTT_TOPIC_LOCK_STATE_LEN] {
    const SUFFIX: &str = MQTT_TOPIC_SUFFIX_LOCK_STATE;

    let mut topic = [0u8; MQTT_TOPIC_LOCK_STATE_LEN];
    let prefix_offset: usize = 0;
    let device_id_offset: usize = TOPIC_PREFIX.len();
    let suffix_offset: usize = device_id_offset + device_id.len();

    topic[prefix_offset..device_id_offset].copy_from_slice(TOPIC_PREFIX.as_bytes());
    topic[device_id_offset..suffix_offset].copy_from_slice(device_id);
    topic[suffix_offset..].copy_from_slice(SUFFIX.as_bytes());
    topic
}

pub(super) fn mk_sensor_state_topic(device_id: &[u8; 12]) -> [u8; MQTT_TOPIC_SENSOR_STATE_LEN] {
    const SUFFIX: &str = MQTT_TOPIC_SUFFIX_SENSOR_STATE;

    let mut topic = [0u8; MQTT_TOPIC_SENSOR_STATE_LEN];
    let prefix_offset: usize = 0;
    let device_id_offset: usize = TOPIC_PREFIX.len();
    let suffix_offset: usize = device_id_offset + device_id.len();

    topic[prefix_offset..device_id_offset].copy_from_slice(TOPIC_PREFIX.as_bytes());
    topic[device_id_offset..suffix_offset].copy_from_slice(device_id);
    topic[suffix_offset..].copy_from_slice(SUFFIX.as_bytes());
    topic
}

pub(super) fn mk_discovery_topic(device_id: &[u8; 12]) -> [u8; MQTT_TOPIC_DISCOVERY_LEN] {
    const LEN: usize = MQTT_TOPIC_DISCOVERY_PREFIX.len() + 12 + MQTT_TOPIC_DISCOVERY_SUFFIX.len();
    let mut topic = [0u8; LEN];

    let prefix_offset: usize = 0;
    let device_id_offset: usize = MQTT_TOPIC_DISCOVERY_PREFIX.len();
    let suffix_offset: usize = device_id_offset + device_id.len();

    topic[prefix_offset..device_id_offset].copy_from_slice(MQTT_TOPIC_DISCOVERY_PREFIX.as_bytes());
    topic[device_id_offset..suffix_offset].copy_from_slice(device_id);
    topic[suffix_offset..].copy_from_slice(MQTT_TOPIC_DISCOVERY_SUFFIX.as_bytes());
    topic
}
