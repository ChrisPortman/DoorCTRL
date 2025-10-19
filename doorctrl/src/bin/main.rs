#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use core::{
    net::{IpAddr, Ipv4Addr},
    str,
};
use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_net::{
    tcp::client::{TcpClient, TcpClientState},
    Runner, Stack, StackResources,
};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, Sender},
    pubsub::{PubSubChannel, Subscriber},
};
use embassy_time::{Duration, Timer};
use embedded_nal_async::TcpConnect;
use esp_alloc as _;
use esp_hal::clock::{Clock, CpuClock};
use esp_hal::efuse::Efuse;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::rng::Rng;
use esp_hal::timer::{systimer::SystemTimer, timg::TimerGroup};

use esp_wifi::{
    init,
    wifi::{ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiState},
    EspWifiController,
};

use doorctrl::mk_static;
use doorctrl::ws2812::{LED, WS2812B};
use doorctrl::{
    door::Door,
    state::{AnyState, LockState},
};
use doorctrl::{hass::MQTTContext, web::HttpService};

const SOCKET_NUM: usize = 6;
const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

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

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    // generator version: 0.5.0

    rtt_target::rtt_init_defmt!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Init RGB
    let mhz = CpuClock::_80MHz.frequency().as_mhz();
    let led = LED {
        inner: WS2812B::new(peripherals.RMT, mhz, peripherals.GPIO8).expect("create LED failed"),
    };
    spawner.spawn(blink(led)).expect("failed to spawn blink");

    let device_id = mk_static!([u8; 12], mac_to_hex(Efuse::read_base_mac_address()));
    info!("{}", device_id);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);

    info!("Embassy initialized!");

    esp_alloc::heap_allocator!(size: 72 * 1024);

    // Init wifi hardware
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let mut rng = Rng::new(peripherals.RNG);

    let esp_wifi_ctrl = &*mk_static!(
        EspWifiController<'static>,
        init(timg0.timer0, rng.clone()).unwrap()
    );

    let (controller, interfaces) = esp_wifi::wifi::new(&esp_wifi_ctrl, peripherals.WIFI).unwrap();

    let wifi_interface = interfaces.sta;
    spawner.spawn(wifi_connection(controller)).ok();
    info!("WIFI initialized!");

    // Init Network stack
    let config = embassy_net::Config::dhcpv4(Default::default());
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        mk_static!(
            StackResources<SOCKET_NUM>,
            StackResources::<SOCKET_NUM>::new()
        ),
        seed,
    );
    spawner.spawn(net_task(runner)).ok();
    info!("Network initialized");

    // Some task comms
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

    spawner
        .spawn(mqtt_service(
            stack,
            device_id,
            cmd_channel.sender(),
            state_pubsub
                .subscriber()
                .inspect_err(|e| error!("error subscribing to states for mqtt_service: {}", e))
                .unwrap(),
        ))
        .ok();

    for _ in 0..4 {
        info!("starting a web server task");
        if let Err(e) = spawner.spawn(http_server(
            stack,
            cmd_channel.sender(),
            state_pubsub
                .subscriber()
                .inspect_err(|e| error!("error subscribing to states for http_service: {}", e))
                .unwrap(),
        )) {
            error!("error spawning web task: {}", e);
        }
    }

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

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[embassy_executor::task]
async fn wifi_connection(mut controller: WifiController<'static>) -> ! {
    info!("start connection task");
    info!("Device capabilities: {:?}", controller.capabilities());
    loop {
        match esp_wifi::wifi::wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.into(),
                password: PASSWORD.into(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            info!("Starting wifi");
            controller.start_async().await.unwrap();
            info!("Wifi started!");

            info!("Scan");
            let result = controller.scan_n_async(10).await.unwrap();
            for ap in result {
                info!("Found SSID: {}", ap.ssid);
            }
        }
        info!("About to connect...");

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
    stack: Stack<'static>,
    device_id: &'static [u8; 12],
    cmd_channel: Sender<'static, CriticalSectionRawMutex, LockState, 2>,
    mut state_sub: Subscriber<'static, CriticalSectionRawMutex, AnyState, 2, 6, 0>,
) -> ! {
    let mut context = MQTTContext::new(device_id);

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
        let conn = match sock
            .connect(core::net::SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(172, 21, 0, 15)),
                1883,
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

        info!("TCP connection to MQTT");
        if let Err(e) = context.run(conn, &cmd_channel, &mut state_sub).await {
            error!("MQTT session error: {}", e);
        }

        Timer::after(Duration::from_secs(5)).await;
    }
}
#[embassy_executor::task(pool_size = 4)]
async fn http_server(
    stack: Stack<'static>,
    cmd_channel: Sender<'static, CriticalSectionRawMutex, LockState, 2>,
    mut state_sub: Subscriber<'static, CriticalSectionRawMutex, AnyState, 2, 6, 0>,
) -> ! {
    loop {
        stack.wait_link_up().await;
        stack.wait_config_up().await;

        let mut service = HttpService::new();
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
async fn blink(mut led: LED) -> ! {
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
