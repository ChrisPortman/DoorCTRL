use embassy_time::{Duration, Timer};
use esp_hal::gpio::{Level, Output, OutputConfig, OutputPin};
use esp_hal::peripherals::RMT;
use esp_hal::rmt::{Channel, PulseCode, Rmt, Tx, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use esp_hal::Async;

const BRG_MAX_NUM_OF_LEDS: usize = 256;
const BRG_PACKET_SIZE: usize = 24;

#[derive(Debug)]
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
        self.ch.transmit(&data).await?;
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

pub struct LED<'a> {
    pub inner: WS2812B<'a>,
}

impl<'a> LED<'a> {
    pub async fn set_color_rgb(&mut self, r: u8, g: u8, b: u8) -> Result<(), Error> {
        self.inner.set_colors(r, g, b).await
    }

    pub async fn flicker(&mut self, ms: u64) -> Result<(), Error> {
        let [r, g, b] = [self.inner.red, self.inner.green, self.inner.blue];
        self.inner.set_colors(0, 0, 0).await?;
        Timer::after(Duration::from_millis(ms)).await;
        self.inner.set_colors(r, g, b).await
    }
}
