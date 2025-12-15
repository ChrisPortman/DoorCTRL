use defmt::error;
use embassy_futures::select::{self, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::{Level, Output, OutputConfig, OutputPin};
use esp_hal::peripherals::RMT;
use esp_hal::rmt::{Channel, PulseCode, Rmt, Tx, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use esp_hal::Async;

const BRG_MAX_NUM_OF_LEDS: usize = 256;
const BRG_PACKET_SIZE: usize = 24;

#[derive(Debug, defmt::Format)]
pub enum Error {
    TooManyLeds,
    RmtError(esp_hal::rmt::Error),
}

// 'From' trait
impl From<esp_hal::rmt::Error> for Error {
    fn from(err: esp_hal::rmt::Error) -> Self {
        Error::RmtError(err)
    }
}

pub struct WS2812B<'a> {
    red: u8,
    green: u8,
    blue: u8,
    ch: Channel<'a, Async, Tx>,
}

impl<'a> WS2812B<'a> {
    /// Create a WS2812B instance with RGB(0, 0, 0)
    ///
    /// Here's an example:
    ///
    /// ```
    /// let mut led = WS2812B::new(peripherals.RMT, 80, peripherals.GPIO8)?;
    /// ```
    pub fn new<P>(rmt: RMT<'a>, freq_mhz: u32, gpio: P) -> Result<Self, Error>
    where
        P: OutputPin + 'a,
    {
        let rmt = Rmt::new(rmt, Rate::from_mhz(freq_mhz))?.into_async();
        let output: Output<'_> = Output::new(gpio, Level::High, OutputConfig::default());
        let tick_rate: u32 = (freq_mhz * 5) / 100; // 50 ns tick!
        let channel = rmt.channel0.configure_tx(
            output,
            TxChannelConfig::default().with_clk_divider(tick_rate as u8),
        )?;

        Ok(WS2812B {
            red: u8::default(),
            green: u8::default(),
            blue: u8::default(),
            ch: channel,
        })
    }

    pub async fn set_colors(&mut self, r: u8, g: u8, b: u8) -> Result<(), Error> {
        self.red = r;
        self.green = g;
        self.blue = b;

        self.play(1).await
    }

    pub async fn set_red(&mut self, r: u8) -> Result<(), Error> {
        self.set_colors(r, 0, 0).await
    }

    pub async fn set_green(&mut self, g: u8) -> Result<(), Error> {
        self.set_colors(0, g, 0).await
    }

    pub async fn set_blue(&mut self, b: u8) -> Result<(), Error> {
        self.set_colors(0, 0, b).await
    }

    pub async fn play(&mut self, num: usize) -> Result<(), Error> {
        if num >= BRG_MAX_NUM_OF_LEDS - 1 {
            return Err(Error::TooManyLeds);
        }

        // Create final stream of data.
        let mut data: [PulseCode; BRG_PACKET_SIZE * BRG_MAX_NUM_OF_LEDS] =
            [PulseCode::default(); BRG_PACKET_SIZE * BRG_MAX_NUM_OF_LEDS];

        // Create RGB packet. (Always the same for now.)
        let packet = self.build_packet();

        for i in 0..num {
            let index = i * BRG_PACKET_SIZE;
            data[index..(index + BRG_PACKET_SIZE)].copy_from_slice(&packet);
        }

        data[num * BRG_PACKET_SIZE] = PulseCode::end_marker();
        // Slice one index extra to fit the `PulseCode::empty()`;
        self.dispatch(&data[0..((num * BRG_PACKET_SIZE) + 1)])
            .await?;

        Ok(())
    }

    async fn dispatch(&mut self, data: &[PulseCode]) -> Result<(), Error> {
        self.ch.transmit(data).await?;
        Ok(())
    }

    // Reference https://cdn-shop.adafruit.com/datasheets/WS2812.pdf
    // in ns: 700/600
    fn get_bit_one(&self) -> PulseCode {
        PulseCode::new(Level::High, 14, Level::Low, 12)
    }

    // in ns: 350/800
    fn get_bit_zero(&self) -> PulseCode {
        // PulseCode::new(Level::High, 8, Level::Low, 17)
        PulseCode::new(Level::High, 7, Level::Low, 16)
    }

    fn build_packet(&self) -> [PulseCode; BRG_PACKET_SIZE] {
        let mut data: [PulseCode; BRG_PACKET_SIZE] = [PulseCode::default(); BRG_PACKET_SIZE];
        let mut index: usize = 0;

        for byte in &[self.green, self.red, self.blue] {
            for bit_index in (0..8).rev() {
                if (*byte >> bit_index) & 0x01 == 0x01 {
                    data[index] = self.get_bit_one();
                } else {
                    data[index] = self.get_bit_zero();
                }
                index += 1;
            }
        }

        data
    }
}

const LIGHT_INTENSITY_DEFAULT: u8 = 32;

pub static LIGHT_UPDATE: Signal<CriticalSectionRawMutex, LightPattern> = Signal::new();

#[derive(Default)]
pub struct LightColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl LightColor {
    pub fn off() -> Self {
        Self::default()
    }

    pub fn red() -> Self {
        Self::default().with_red(LIGHT_INTENSITY_DEFAULT)
    }

    pub fn green() -> Self {
        Self::default().with_green(LIGHT_INTENSITY_DEFAULT)
    }

    pub fn blue() -> Self {
        Self::default().with_blue(LIGHT_INTENSITY_DEFAULT)
    }

    pub fn amber() -> Self {
        Self::default()
            .with_red(LIGHT_INTENSITY_DEFAULT)
            .with_green(16)
    }

    fn with_red(mut self, r: u8) -> Self {
        self.r = r;
        self
    }
    fn with_green(mut self, g: u8) -> Self {
        self.g = g;
        self
    }
    fn with_blue(mut self, b: u8) -> Self {
        self.b = b;
        self
    }
}

pub enum LightPattern {
    Off,
    Solid(LightColor),
    // Blink(color, on_time, off_time)
    Blink(LightColor, Duration, Duration),
    BlinkCode(LightColor, u8),
}

pub struct Light<'a> {
    pub inner: WS2812B<'a>,
}

impl<'a> Light<'a> {
    pub async fn update(update: LightPattern) {
        LIGHT_UPDATE.signal(update);
    }

    pub async fn run(&mut self, initial: LightPattern) -> ! {
        let mut pattern = initial;

        loop {
            match self.do_pattern(pattern).await {
                Ok(None) => {
                    pattern = LIGHT_UPDATE.wait().await;
                    continue;
                }
                Ok(Some(next)) => {
                    pattern = next;
                    continue;
                }
                Err(e) => {
                    error!(
                        "error setting light pattern: {}.  suspending light for 5 seconds",
                        e
                    );
                    pattern = LightPattern::Off;
                    Timer::after(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn do_pattern(&mut self, pattern: LightPattern) -> Result<Option<LightPattern>, Error> {
        match pattern {
            LightPattern::Off => self.set_color(&LightColor::off()).await?,
            LightPattern::Solid(c) => self.set_color(&c).await?,
            LightPattern::Blink(c, on, off) => loop {
                self.set_color(&c).await?;
                if let Some(pat) = self.wait(on).await {
                    return Ok(Some(pat));
                }
                self.inner.set_colors(0, 0, 0).await?;
                if let Some(pat) = self.wait(off).await {
                    return Ok(Some(pat));
                }
            },
            LightPattern::BlinkCode(c, count) => {
                let short = Duration::from_millis(300);
                let long = Duration::from_millis(1000);

                loop {
                    for _ in 0..count {
                        self.set_color(&LightColor::off()).await?;
                        if let Some(pat) = self.wait(short).await {
                            return Ok(Some(pat));
                        }
                        self.set_color(&c).await?;
                        if let Some(pat) = self.wait(short).await {
                            return Ok(Some(pat));
                        }
                    }

                    self.set_color(&LightColor::off()).await?;
                    if let Some(pat) = self.wait(long).await {
                        return Ok(Some(pat));
                    }
                }
            }
        };

        Ok(None)
    }

    async fn wait(&self, dur: Duration) -> Option<LightPattern> {
        match select(Timer::after(dur), LIGHT_UPDATE.wait()).await {
            select::Either::First(_) => None,
            select::Either::Second(update) => Some(update),
        }
    }

    pub async fn set_color(&mut self, color: &LightColor) -> Result<(), Error> {
        self.inner.set_colors(color.r, color.g, color.b).await
    }
}
