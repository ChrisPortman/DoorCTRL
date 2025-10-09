// Communicate with Home Assistant using MQTT as described
// https://www.home-assistant.io/integrations/mqtt/
#![allow(dead_code)]

pub mod discover;
mod topic;

use core::str;
use defmt::{error, info};

use embassy_futures::select;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Sender, pubsub::Subscriber,
};
use embassy_time::{Duration, Timer};
use embedded_io_async::{Read, Write};

use rust_mqtt::{
    client::{client::MqttClient, client_config::ClientConfig},
    packet::v5::{publish_packet::QualityOfService, reason_codes::ReasonCode},
    utils::rng_generator::CountingRng,
};
use serde_json_core::to_slice;

use crate::state::{AnyState, DoorState, LockState};
use discover::Discovery;
use topic::{
    mk_availability_topic, mk_discovery_topic, mk_lock_cmd_topic, mk_lock_state_topic,
    mk_sensor_state_topic,
};

const MQTT_PAYLOAD_AVAILABLE: &str = "online";
const MQTT_PAYLOAD_NOT_AVAILABLE: &str = "offline";
const MQTT_PAYLOAD_LOCK: &str = "LOCK";
const MQTT_PAYLOAD_UNLOCK: &str = "UNLOCK";
const MQTT_STATE_LOCKED: &str = "LOCKED";
const MQTT_STATE_UNLOCKED: &str = "UNLOCKED";
const MQTT_STATE_OFF: &str = "OFF";
const MQTT_STATE_ON: &str = "ON";

const BUFFER_LEN: usize = 1024;
const MQTT_KEEPALIVE: u64 = 60;

pub fn make_buffers() -> [[u8; BUFFER_LEN]; 2] {
    let rx = [0u8; BUFFER_LEN];
    let tx = [0u8; BUFFER_LEN];
    [rx, tx]
}

pub struct MQTTContext<'a> {
    device_id: &'a [u8; 12],
    discovery_topic: [u8; topic::MQTT_TOPIC_DISCOVERY_LEN],
    availability_topic: [u8; topic::MQTT_TOPIC_AVAILABILITY_LEN],
    lock_cmd_topic: [u8; topic::MQTT_TOPIC_LOCK_COMMAND_LEN],
    lock_state_topic: [u8; topic::MQTT_TOPIC_LOCK_STATE_LEN],
    sensor_state_topic: [u8; topic::MQTT_TOPIC_SENSOR_STATE_LEN],
}

impl<'a> MQTTContext<'a> {
    pub fn new(device_id: &'a [u8; 12]) -> Self {
        Self {
            device_id: device_id,
            discovery_topic: mk_discovery_topic(device_id),
            availability_topic: mk_availability_topic(device_id),
            lock_cmd_topic: mk_lock_cmd_topic(device_id),
            lock_state_topic: mk_lock_state_topic(device_id),
            sensor_state_topic: mk_sensor_state_topic(device_id),
        }
    }

    pub async fn connect<T: Read + Write>(
        &self,
        client: &mut MqttClient<'a, T, 3, CountingRng>,
    ) -> Result<(), ReasonCode> {
        client.connect_to_broker().await?;

        let discovery_payload = Discovery::new(
            str::from_utf8(&self.availability_topic).unwrap(),
            str::from_utf8(&self.lock_state_topic).unwrap(),
            str::from_utf8(&self.lock_cmd_topic).unwrap(),
            str::from_utf8(&self.sensor_state_topic).unwrap(),
        );

        let mut discovery_payload_json = [0u8; 1024];
        let len = to_slice(&discovery_payload, &mut discovery_payload_json[..]).unwrap();
        if let Err(e) = client
            .send_message(
                str::from_utf8(&self.discovery_topic).unwrap(),
                &discovery_payload_json[..len],
                QualityOfService::QoS1,
                false,
            )
            .await
        {
            error!("failed to send discovery payload: {}", e);
            return Err(e);
        }
        info!("discovery sent to {}", self.discovery_topic);
        info!(
            "{}",
            str::from_utf8(&discovery_payload_json[..len]).unwrap()
        );

        if let Err(e) = client
            .send_message(
                str::from_utf8(&self.availability_topic).unwrap(),
                MQTT_PAYLOAD_AVAILABLE.as_bytes(),
                QualityOfService::QoS1,
                true,
            )
            .await
        {
            error!("failed to send availability message: {}", e);
            return Err(e);
        }

        Ok(())
    }

    pub async fn run<T: Read + Write>(
        &mut self,
        sock: T,
        cmd_channel: &Sender<'static, CriticalSectionRawMutex, LockState, 2>,
        state_sub: &mut Subscriber<'static, CriticalSectionRawMutex, AnyState, 2, 6, 0>,
    ) -> Result<(), ReasonCode> {
        // subscribe to the lock command topic
        // listen for door state changes
        // listen for lock state changes
        // select across all the above, and handle.

        let mut config = ClientConfig::<3, _>::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(20000),
        );
        config.add_max_subscribe_qos(rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1);
        config.add_client_id("doorctrl");
        config.add_username("mqttuser");
        config.add_password("TF2GVZVfQ-XeiJa-VC6R");
        config.add_will(
            str::from_utf8(&self.availability_topic).unwrap(),
            MQTT_PAYLOAD_NOT_AVAILABLE.as_bytes(),
            false,
        );
        config.max_packet_size = 1024;

        let [mut rx, mut tx] = make_buffers();

        let mut client = MqttClient::new(sock, &mut tx, BUFFER_LEN, &mut rx, BUFFER_LEN, config);
        self.connect(&mut client).await?;

        if let Err(e) = client
            .subscribe_to_topic(str::from_utf8(&self.lock_cmd_topic).unwrap())
            .await
        {
            error!("failed to subscribe to lock command topic: {}", e);
            return Err(e);
        }

        loop {
            let work = select::select3(
                client.receive_message(),
                state_sub.next_message_pure(),
                Timer::after(Duration::from_secs(MQTT_KEEPALIVE)),
            )
            .await;

            match work {
                select::Either3::First(Ok((topic, data))) => {
                    info!("received command on topic {}: {}", topic, data);
                    if data == MQTT_PAYLOAD_LOCK.as_bytes() {
                        info!("received lock command on topic {}: {}", topic, data);
                        cmd_channel.clear();
                        cmd_channel.send(LockState::Locked).await;
                    } else if data == MQTT_PAYLOAD_UNLOCK.as_bytes() {
                        info!("received unlock command on topic {}: {}", topic, data);
                        cmd_channel.clear();
                        cmd_channel.send(LockState::Unlocked).await;
                    } else {
                        error!("recieved unknown lock command");
                    }
                }
                select::Either3::First(Err(e)) => {
                    error!("error receiving from mqtt: {}", e);
                    return Err(e);
                }
                select::Either3::Second(AnyState::LockState(LockState::Locked)) => {
                    info!("sending door locked to mqtt");
                    if let Err(e) = client
                        .send_message(
                            str::from_utf8(&self.lock_state_topic).unwrap(),
                            MQTT_STATE_LOCKED.as_bytes(),
                            QualityOfService::QoS1,
                            false,
                        )
                        .await
                    {
                        error!("failed to send locked state payload: {}", e);
                        return Err(e);
                    }
                }
                select::Either3::Second(AnyState::LockState(LockState::Unlocked)) => {
                    info!("sending door unlocked to mqtt");
                    if let Err(e) = client
                        .send_message(
                            str::from_utf8(&self.lock_state_topic).unwrap(),
                            MQTT_STATE_UNLOCKED.as_bytes(),
                            QualityOfService::QoS1,
                            false,
                        )
                        .await
                    {
                        error!("failed to send unlocked state payload: {}", e);
                        return Err(e);
                    }
                }
                select::Either3::Second(AnyState::DoorState(DoorState::Open)) => {
                    info!("sending door open to mqtt");
                    if let Err(e) = client
                        .send_message(
                            str::from_utf8(&self.sensor_state_topic).unwrap(),
                            MQTT_STATE_ON.as_bytes(),
                            QualityOfService::QoS1,
                            false,
                        )
                        .await
                    {
                        error!("failed to send door state open payload: {}", e);
                        return Err(e);
                    }
                }
                select::Either3::Second(AnyState::DoorState(DoorState::Closed)) => {
                    info!("sending door closed to mqtt");
                    if let Err(e) = client
                        .send_message(
                            str::from_utf8(&self.sensor_state_topic).unwrap(),
                            MQTT_STATE_OFF.as_bytes(),
                            QualityOfService::QoS1,
                            false,
                        )
                        .await
                    {
                        error!("failed to send door state closed payload: {}", e);
                        return Err(e);
                    }
                }
                select::Either3::Third(_) => {
                    info!("sending keepalive");
                    if let Err(e) = client.send_ping().await {
                        error!("error sending pingL {}", e);
                        return Err(e);
                    }
                }
            }
        }
    }
}
