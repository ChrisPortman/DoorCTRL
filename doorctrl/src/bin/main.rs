#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use core::{
    net::{IpAddr, Ipv4Addr},
    ops::DerefMut,
    str::FromStr,
};
use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_net::{
    tcp::client::{TcpClient, TcpClientState},
    Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4,
};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, Sender},
    mutex::Mutex,
    pubsub::{PubSubChannel, Subscriber},
};
use embassy_time::{Duration, Timer};
use embedded_nal_async::TcpConnect;
use esp_alloc as _;
use esp_bootloader_esp_idf::partitions::{self, FlashRegion, PartitionEntry};
use esp_hal::clock::{Clock, CpuClock};
use esp_hal::efuse::Efuse;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
#[cfg(target_arch = "riscv32")]
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;

use esp_radio::{
    wifi::{
        AccessPointConfig, AuthMethod, ClientConfig, ModeConfig, ScanConfig, WifiApState,
        WifiController, WifiDevice, WifiEvent, WifiStaState,
    },
    Controller,
};
use esp_storage::FlashStorage;
use heapless::Vec;

use conf::ConfigV1;
use doorctrl::mk_static;

use doorctrl::ws2812::{LED, WS2812B};
use doorctrl::{
    door::Door,
    state::{AnyState, LockState},
};
use doorctrl::{hass::MQTTContext, web::HttpService};

const SOCKET_NUM: usize = 8;
// const SSID: &str = env!("SSID");
// const PASSWORD: &str = env!("PASSWORD");

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

fn u8_to_hex(u: u8) -> [u8; 2] {
    fn nybble_to_hex(n: u8) -> u8 {
        if n < 10 {
            // 40 is ascii 0
            return 48 + n;
        }

        // 97 is ascii 'a'
        97 + (n - 10)
    }

    let upper = u >> 4;
    let lower = u << 4 >> 4;

    [nybble_to_hex(upper), nybble_to_hex(lower)]
}

fn mac_to_hex(mac: [u8; 6]) -> [u8; 12] {
    let mut hex: [u8; 12] = [0; 12];
    for idx in 0..6 {
        let [upper, lower] = u8_to_hex(mac[idx]);
        hex[idx * 2] = upper;
        hex[idx * 2 + 1] = lower;
    }
    hex
}

type Storage = &'static Mutex<CriticalSectionRawMutex, FlashRegion<'static, FlashStorage<'static>>>;

fn prepare_flash(flash: &'static mut FlashStorage<'static>) -> Storage {
    let partition_buf = mk_static!(
        [u8; partitions::PARTITION_TABLE_MAX_LEN],
        [0u8; partitions::PARTITION_TABLE_MAX_LEN]
    );
    let partition_info = partitions::read_partition_table(flash, partition_buf).unwrap();
    let nvs = mk_static!(
        PartitionEntry<'static>,
        partition_info
            .find_partition(partitions::PartitionType::Data(
                partitions::DataPartitionSubType::Nvs,
            ))
            .unwrap()
            .unwrap()
    );
    let nvs_part = nvs.as_embedded_storage(flash);

    mk_static!(
        Mutex<CriticalSectionRawMutex, FlashRegion<'_, FlashStorage<'_>>>,
        Mutex::new(nvs_part)
    )
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let hal_config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(hal_config);
    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    #[cfg(target_arch = "riscv32")]
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(
        timg0.timer0,
        #[cfg(target_arch = "riscv32")]
        sw_int.software_interrupt0,
    );
    info!("Embassy initialized!");

    // Some task comm
    // cmd_channel is for processing incomming command from external sources (i.e. lock/unlock)
    let cmd_channel = mk_static!(
        Channel::<CriticalSectionRawMutex, LockState, 2>,
        Channel::<CriticalSectionRawMutex, LockState, 2>::new()
    );
    // state_pubsub is for eminating changes in state as they are detected
    let state_pubsub = mk_static!(
        PubSubChannel::<CriticalSectionRawMutex, AnyState, 2, 6, 0>,
        PubSubChannel::<CriticalSectionRawMutex, AnyState, 2, 6, 0>::new()
    );

    // Real Time Trasfer protocol for probe-rs logging etc.
    rtt_target::rtt_init_defmt!();

    // Flash Memory
    let flash = mk_static!(FlashStorage, FlashStorage::new(peripherals.FLASH));
    let storage = prepare_flash(flash);

    // Init RGB
    let mhz = CpuClock::_80MHz.frequency().as_mhz();
    let led = LED {
        inner: WS2812B::new(peripherals.RMT, mhz, peripherals.GPIO8).expect("create LED failed"),
    };
    spawner.spawn(blink(led)).expect("failed to spawn blink");

    let device_id = mk_static!([u8; 12], mac_to_hex(Efuse::read_base_mac_address()));
    info!("{}", device_id);

    // Init the door
    let lock_pin = Output::new(peripherals.GPIO1, Level::Low, OutputConfig::default());
    let reed_pin = Input::new(
        peripherals.GPIO2,
        InputConfig::default().with_pull(Pull::Up),
    );
    let door = Door::new(
        lock_pin,
        reed_pin,
        cmd_channel.receiver(),
        state_pubsub.immediate_publisher(),
    );
    spawner.spawn(door_service(door)).ok();

    // Init wifi hardware
    let esp_radio_ctrl = &*mk_static!(Controller<'static>, esp_radio::init().unwrap());
    let (controller, interfaces) =
        esp_radio::wifi::new(&esp_radio_ctrl, peripherals.WIFI, Default::default()).unwrap();
    let mut wifi_interface = interfaces.sta;

    let mut locked_storage = storage.lock().await;
    let config = match ConfigV1::load(locked_storage.deref_mut()) {
        Ok(c) => Some(c),
        Err(e) => {
            error!(
                "error loading config: {:?}. proceding as if unconfigured",
                e
            );
            None
        }
    };
    drop(locked_storage);

    let mut net_config = embassy_net::Config::dhcpv4(Default::default());

    match config {
        Some(c) => {
            spawner
                .spawn(wifi_client(controller, c.wifi_ssid, c.wifi_pass))
                .ok();
        }
        None => {
            spawner.spawn(wifi_ap(controller)).ok();
            net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
                address: Ipv4Cidr::new(Ipv4Addr::new(192, 168, 0, 1), 24),
                gateway: None,
                dns_servers: Vec::<_, 3>::new(),
            });
            wifi_interface = interfaces.ap;
        }
    }

    // Init Network stack
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        net_config,
        mk_static!(
            StackResources<SOCKET_NUM>,
            StackResources::<SOCKET_NUM>::new()
        ),
        seed,
    );

    spawner.spawn(net_task(runner)).ok();
    info!("Network initialized");

    if let Some(c) = config {
        spawner
            .spawn(mqtt_service(
                c.mqtt_host,
                c.mqtt_user,
                c.mqtt_pass,
                stack,
                device_id,
                cmd_channel.sender(),
                state_pubsub
                    .subscriber()
                    .inspect_err(|e| error!("error subscribing to states for mqtt_service: {}", e))
                    .unwrap(),
            ))
            .ok();
    }

    let config = config.unwrap_or(ConfigV1::default());

    for _ in 0..4 {
        info!("starting a web server task");
        if let Err(e) = spawner.spawn(http_server(
            config,
            stack,
            storage,
            cmd_channel.sender(),
            state_pubsub
                .subscriber()
                .inspect_err(|e| error!("error subscribing to states for http_service: {}", e))
                .unwrap(),
        )) {
            error!("error spawning web task: {}", e);
        }
    }

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[embassy_executor::task]
async fn wifi_ap(mut controller: WifiController<'static>) -> ! {
    info!("Device capabilities: {:?}", controller.capabilities());
    loop {
        match esp_radio::wifi::ap_state() {
            WifiApState::Started => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::ApStop).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let ap_config = AccessPointConfig::default()
                .with_ssid("DoorControl".into())
                .with_auth_method(AuthMethod::Wpa2Personal)
                .with_password("new_door_control".into());
            let client_config = ModeConfig::AccessPoint(ap_config);

            if let Err(e) = controller.set_config(&client_config) {
                error!("wifi AP configuration error: {}", e);
            }
            controller.start_async().await.unwrap();
            info!("Wifi AP started!");
        }
    }
}

#[embassy_executor::task]
async fn wifi_client(
    mut controller: WifiController<'static>,
    ssid: conf::ConfigV1Value,
    pass: conf::ConfigV1Value,
) -> ! {
    loop {
        match esp_radio::wifi::sta_state() {
            WifiStaState::Connected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(ssid.as_str().into())
                    .with_password(pass.as_str().into()),
            );

            if let Err(e) = controller.set_config(&client_config) {
                error!("wifi station configuration error: {}", e);
            }

            controller.start_async().await.unwrap();

            let scan_config = ScanConfig::default().with_max(10);
            let result = controller
                .scan_with_config_async(scan_config)
                .await
                .unwrap();
            for ap in result {
                info!("Found SSID: {}", ap.ssid);
            }
        }
        info!("WIFI connecting ...");

        match controller.connect_async().await {
            Ok(_) => info!("Wifi connected!"),
            Err(e) => {
                info!("Failed to connect to wifi: {:?}", e);
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn mqtt_service(
    mqtt_host: conf::ConfigV1Value,
    mqtt_user: conf::ConfigV1Value,
    mqtt_pass: conf::ConfigV1Value,
    stack: Stack<'static>,
    device_id: &'static [u8; 12],
    cmd_channel: Sender<'static, CriticalSectionRawMutex, LockState, 2>,
    mut state_sub: Subscriber<'static, CriticalSectionRawMutex, AnyState, 2, 6, 0>,
) -> ! {
    let mut context = MQTTContext::new(device_id, mqtt_user.as_str(), mqtt_pass.as_str());

    let mqtt_ipaddr = match Ipv4Addr::from_str(mqtt_host.as_str()) {
        Ok(i) => i,
        Err(_) => {
            loop {
                // Never progress...
                error!("mqtt host is not a valid IP address");
                Timer::after(Duration::from_secs(3600)).await;
            }
        }
    };

    loop {
        stack.wait_link_up().await;
        stack.wait_config_up().await;
        info!(
            "MQTT: IP config applied {}",
            stack.config_v4().unwrap().address,
        );
        info!("MQTT: Wifi connected");

        let state = TcpClientState::<3, 1024, 1024>::new();
        let sock = TcpClient::new(stack, &state);
        info!("MQTT: connecting to {}", mqtt_ipaddr);
        let conn = match sock
            .connect(core::net::SocketAddr::new(IpAddr::V4(mqtt_ipaddr), 1883))
            .await
        {
            Ok(c) => c,
            Err(e) => {
                info!("failed to connect MQTT: {}", e);
                Timer::after(Duration::from_secs(5)).await;
                continue;
            }
        };

        info!("TCP connection to MQTT");
        if let Err(e) = context.run(conn, &cmd_channel, &mut state_sub).await {
            error!("MQTT session error: {}", e);
        }

        Timer::after(Duration::from_secs(5)).await;
    }
}
#[embassy_executor::task(pool_size = 4)]
async fn http_server(
    config: ConfigV1,
    stack: Stack<'static>,
    storage: Storage,
    cmd_channel: Sender<'static, CriticalSectionRawMutex, LockState, 2>,
    mut state_sub: Subscriber<'static, CriticalSectionRawMutex, AnyState, 2, 6, 0>,
) -> ! {
    loop {
        stack.wait_link_up().await;
        stack.wait_config_up().await;

        let mut service = HttpService::new(config, storage);
        if let Err(e) = service.run(stack, &cmd_channel, &mut state_sub).await {
            error!(
                "web server returned an error. Will restart in 5 secs: {}",
                e
            );
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
async fn door_service(
    mut door: Door<'static, Output<'static>, Input<'static>, CriticalSectionRawMutex>,
) -> ! {
    loop {
        door.run().await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn blink(mut led: LED<'static>) -> ! {
    info!("blinking led");
    let rgbs: [[u8; 3]; 6] = [
        [1, 0, 0],
        [1, 1, 0],
        [0, 1, 0],
        [0, 1, 1],
        [0, 0, 1],
        [1, 0, 1],
    ];

    let intensity: u8 = 16;

    loop {
        for rgb in rgbs.iter() {
            let [r, g, b] = rgb;
            led.set_color_rgb(*r * intensity, *g * intensity, *b * intensity)
                .await
                .expect("configuring led failed");
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}
