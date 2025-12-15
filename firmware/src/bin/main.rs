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
use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_futures::select;
use embassy_net::{
    tcp::{
        client::{TcpClient, TcpClientState, TcpConnection},
        TcpSocket,
    },
    IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4,
};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, mutex::Mutex,
    pubsub::PubSubChannel,
};
use embassy_time::{Duration, Timer};

use embedded_nal_async::TcpConnect;
use embedded_storage::nor_flash::NorFlash;
use embedded_tls::{Aes128GcmSha256, NoVerify, TlsConfig, TlsConnection, TlsContext};

use esp_alloc as _;
use esp_bootloader_esp_idf::partitions::{self, FlashRegion, PartitionEntry};
use esp_hal::clock::{Clock, CpuClock};
use esp_hal::efuse::Efuse;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
#[cfg(target_arch = "riscv32")]
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::rng::{Rng, Trng};
use esp_hal::timer::timg::TimerGroup;

use esp_radio::{
    wifi::{
        AccessPointConfig, AuthMethod, ClientConfig, Interfaces, ModeConfig, ScanConfig,
        WifiApState, WifiController, WifiDevice, WifiEvent, WifiStaState,
    },
    Controller,
};
use esp_storage::FlashStorage;
use heapless::Vec;

use doorctrl::config::{ConfigV1, ConfigV1Value};
use doorctrl::door::Door;
use doorctrl::hass::MQTTContext;
use doorctrl::state::{AnyState, LockState};

use firmware::web::HttpClientHandler;
use firmware::ws2812::{Light, LightColor, LIGHT_UPDATE, WS2812B};
use firmware::{mk_static, ws2812::LightPattern};

const SOCKET_NUM: usize = 8;

// cmd_channel is for processing incomming command from external sources (i.e. lock/unlock)
static CMD_CHANNEL: Channel<CriticalSectionRawMutex, LockState, 2> =
    Channel::<CriticalSectionRawMutex, LockState, 2>::new();
// state_pubsub is for eminating changes in state as they are detected
static STATE_PUBSUB: PubSubChannel<CriticalSectionRawMutex, AnyState, 2, 6, 0> =
    PubSubChannel::<CriticalSectionRawMutex, AnyState, 2, 6, 0>::new();

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
    // Real Time Trasfer protocol for probe-rs logging etc.
    rtt_target::rtt_init_defmt!();

    let hal_config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(hal_config);
    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    #[cfg(target_arch = "riscv32")]
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(
        timg0.timer0,
        #[cfg(target_arch = "riscv32")]
        sw_int.software_interrupt0,
    );

    // Init RGB
    let light = Light {
        inner: WS2812B::new(
            peripherals.RMT,
            CpuClock::_80MHz.frequency().as_mhz(),
            peripherals.GPIO8,
        )
        .expect("create LED failed"),
    };
    spawner.spawn(blink(light)).expect("failed to spawn blink");
    LIGHT_UPDATE.signal(LightPattern::Solid(LightColor::red()));

    // Flash Memory
    let flash = mk_static!(FlashStorage, FlashStorage::new(peripherals.FLASH));
    let storage = prepare_flash(flash);

    let rst_pin = Input::new(
        peripherals.GPIO3,
        InputConfig::default().with_pull(Pull::Up),
    );

    // Init the door
    let lock_pin = Output::new(peripherals.GPIO1, Level::Low, OutputConfig::default());
    let reed_pin = Input::new(
        peripherals.GPIO2,
        InputConfig::default().with_pull(Pull::Up),
    );
    let door = Door::new(
        lock_pin,
        reed_pin,
        CMD_CHANNEL.receiver(),
        STATE_PUBSUB.immediate_publisher(),
    );
    spawner.spawn(door_service(door)).ok();

    // Init wifi hardware
    let esp_radio_ctrl = &*mk_static!(Controller<'static>, esp_radio::init().unwrap());
    let (controller, interfaces) =
        esp_radio::wifi::new(esp_radio_ctrl, peripherals.WIFI, Default::default()).unwrap();

    let mut locked_storage = storage.lock().await;
    let config = ConfigV1::load(locked_storage.deref_mut());
    drop(locked_storage);

    match config {
        Ok(cfg) => {
            info!("config ready, entering normal mode");
            normal_mode(spawner, cfg, controller, interfaces, storage, rst_pin).await
        }
        Err(e) => {
            warn!("config not ready ({}), entering setup mode", e);
            setup_mode(spawner, controller, interfaces, storage).await;
        }
    };

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

async fn normal_mode(
    spawner: Spawner,
    config: ConfigV1,
    controller: WifiController<'static>,
    interfaces: Interfaces<'static>,
    storage: Storage,
    rst_pin: Input<'static>,
) {
    if let Err(e) = spawner.spawn(factory_resetter(rst_pin, storage)) {
        error!("error spawning reset monitor: {}", e);
    }

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let device_id = mk_static!([u8; 12], mac_to_hex(Efuse::read_base_mac_address()));
    let wifi_interface = interfaces.sta;
    let net_config = embassy_net::Config::dhcpv4(Default::default());

    spawner
        .spawn(wifi_client(controller, config.wifi_ssid, config.wifi_pass))
        .ok();

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

    stack.wait_link_up().await;
    info!("Wifi connected");
    LIGHT_UPDATE.signal(LightPattern::Blink(
        LightColor::green(),
        Duration::from_millis(500),
        Duration::from_millis(500),
    ));

    stack.wait_config_up().await;
    info!("IP config applied {}", stack.config_v4().unwrap().address);

    if let Err(e) = spawner.spawn(mqtt_service(device_id, config, stack)) {
        error!("error spanning MQTT client: {}", e);
    }

    let cmd_sender = CMD_CHANNEL.sender();

    let http_server = mk_static!(
        weblite::server::Server::<HttpClientHandler>,
        weblite::server::Server::<_>::new(HttpClientHandler::new(
            firmware::web::HttpServiceState {
                storage,
                config,
                door_state: None,
                lock_state: None,
            },
            cmd_sender,
            &STATE_PUBSUB,
        ))
    );

    for _ in 0..4 {
        info!("starting a web server task");
        if let Err(e) = spawner.spawn(http_connection(stack, http_server)) {
            error!("error spawning web task: {}", e);
        }
    }
}

async fn setup_mode(
    spawner: Spawner,
    controller: WifiController<'static>,
    interfaces: Interfaces<'static>,
    storage: Storage,
) {
    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let wifi_interface = interfaces.ap;
    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Addr::new(192, 168, 0, 1), 24),
        gateway: None,
        dns_servers: Vec::<_, 3>::new(),
    });
    let config = ConfigV1::default();

    spawner.spawn(wifi_ap(controller)).ok();

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

    let cmd_sender = CMD_CHANNEL.sender();

    let http_server = mk_static!(
        weblite::server::Server::<HttpClientHandler>,
        weblite::server::Server::<_>::new(HttpClientHandler::new(
            firmware::web::HttpServiceState {
                storage,
                config,
                door_state: None,
                lock_state: None,
            },
            cmd_sender,
            &STATE_PUBSUB,
        ))
    );

    for _ in 0..4 {
        info!("starting a web server task");
        if let Err(e) = spawner.spawn(http_connection(stack, http_server)) {
            error!("error spawning web task: {}", e);
        }
    }
}

#[embassy_executor::task]
async fn wifi_ap(mut controller: WifiController<'static>) -> ! {
    info!("Device capabilities: {:?}", controller.capabilities());
    loop {
        if esp_radio::wifi::ap_state() == WifiApState::Started {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::ApStop).await;
            Timer::after(Duration::from_millis(5000)).await
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
            LIGHT_UPDATE.signal(LightPattern::Blink(
                LightColor::amber(),
                Duration::from_millis(500),
                Duration::from_millis(500),
            ));
        }
    }
}

#[embassy_executor::task]
async fn wifi_client(
    mut controller: WifiController<'static>,
    ssid: ConfigV1Value,
    pass: ConfigV1Value,
) -> ! {
    loop {
        if esp_radio::wifi::sta_state() == WifiStaState::Connected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await
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
            Ok(_) => {
                info!("Wifi connected!");
                LIGHT_UPDATE.signal(LightPattern::Solid(LightColor::amber()));
            }
            Err(e) => {
                info!("Failed to connect to wifi: {:?}", e);
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn mqtt_service(device_id: &'static [u8; 12], config: ConfigV1, stack: Stack<'static>) -> ! {
    let mut context = MQTTContext::new(
        device_id,
        config.device_name.as_str(),
        config.mqtt_user.as_str(),
        config.mqtt_pass.as_str(),
    );

    let mqtt_ipaddr = match Ipv4Addr::from_str(config.mqtt_host.as_str()) {
        Ok(i) => i,
        Err(_) => {
            loop {
                // Never progress...
                error!("mqtt host is not a valid IP address");
                Timer::after(Duration::from_secs(3600)).await;
            }
        }
    };

    let mut tls_read_buf = [0u8; 16640];
    let mut tls_write_buf = [0u8; 16640];

    let state = TcpClientState::<3, 1024, 1024>::new();
    loop {
        stack.wait_link_up().await;
        stack.wait_config_up().await;

        let sock = TcpClient::new(stack, &state);
        info!("MQTT: connecting to {}", mqtt_ipaddr);
        let conn = match sock
            .connect(core::net::SocketAddr::new(
                IpAddr::V4(mqtt_ipaddr),
                config.mqtt_port,
            ))
            .await
        {
            Ok(c) => c,
            Err(e) => {
                info!("failed to connect MQTT: {}", e);
                Timer::after(Duration::from_secs(5)).await;
                continue;
            }
        };

        match config.mqtt_tls {
            true => {
                let mut rng = Trng::try_new().unwrap();
                let tls_config = TlsConfig::new().with_server_name(config.mqtt_host.as_str());
                let mut tls_conn =
                    TlsConnection::<TcpConnection<'_, 3, 1024, 1024>, Aes128GcmSha256>::new(
                        conn,
                        tls_read_buf.as_mut_slice(),
                        tls_write_buf.as_mut_slice(),
                    );

                match tls_conn
                    .open::<Trng, NoVerify>(TlsContext::new(&tls_config, &mut rng))
                    .await
                {
                    Err(e) => error!("could not establish TLS connection to MQTT broker: {}", e),
                    Ok(()) => {
                        info!("TLS connection to MQTT");

                        LIGHT_UPDATE.signal(LightPattern::Solid(LightColor::green()));
                        if let Err(e) = context
                            .run(
                                tls_conn,
                                &CMD_CHANNEL.sender(),
                                &mut STATE_PUBSUB.subscriber().unwrap(),
                            )
                            .await
                        {
                            error!("MQTT session error: {}", e);
                        }
                    }
                }
            }
            false => {
                info!("TCP connection to MQTT");
                LIGHT_UPDATE.signal(LightPattern::Solid(LightColor::green()));
                if let Err(e) = context
                    .run(
                        conn,
                        &CMD_CHANNEL.sender(),
                        &mut STATE_PUBSUB.subscriber().unwrap(),
                    )
                    .await
                {
                    error!("MQTT session error: {}", e);
                }
            }
        }

        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task(pool_size = 4)]
async fn http_connection(
    stack: Stack<'static>,
    http_server: &'static weblite::server::Server<HttpClientHandler>,
) -> ! {
    let mut tx_buf = [0u8; 1024];
    let mut rx_buf = [0u8; 1024];
    let mut http_buff = [0u8; 1024];

    loop {
        stack.wait_link_up().await;
        stack.wait_config_up().await;

        let mut conn = TcpSocket::new(stack, rx_buf.as_mut_slice(), tx_buf.as_mut_slice());
        if let Err(e) = conn
            .accept(IpListenEndpoint {
                addr: None,
                port: 80,
            })
            .await
        {
            error!("error accepting http connection: {}", e);
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        if let Err(e) = http_server.serve(&mut conn, http_buff.as_mut_slice()).await {
            error!("HTTP error: {}", e);
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
async fn factory_resetter(mut pin: Input<'static>, storage: Storage) -> ! {
    loop {
        pin.wait_for_low().await;
        info!("reset button pushed");
        let action =
            select::select(pin.wait_for_high(), Timer::after(Duration::from_secs(5))).await;

        match action {
            select::Either::First(_) => {
                // Pin went high (button released) before 5 secs
                info!("reset button released before timeout, not resetting");
            }
            select::Either::Second(_) => {
                // Held low for long enough. Delete config and reset.
                info!("reset button held for 5 seconds, resetting");

                {
                    let mut locked_storage = storage.lock().await;
                    if let Err(e) = locked_storage.erase(0, 4096) {
                        error!("failed to erase storage before reset: {}", e);
                    }
                }

                esp_hal::system::software_reset();
            }
        }
    }
}

#[embassy_executor::task]
async fn blink(mut led: Light<'static>) -> ! {
    info!("initializing LED");
    led.run(LightPattern::Off).await;
}
