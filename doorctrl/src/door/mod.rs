use defmt::{error, info};

use embassy_futures::select;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::{channel::Receiver, pubsub::ImmediatePublisher};
use embedded_hal::digital::{Error, ErrorType, InputPin, OutputPin, PinState, StatefulOutputPin};
use embedded_hal_async::digital::Wait;

use crate::state::{AnyState, DoorState, LockState};

pub struct Door<'a, L, R, M>
where
    L: OutputPin + StatefulOutputPin,
    R: InputPin + Wait,
    M: RawMutex,
{
    cmd_channel: Receiver<'a, M, LockState, 2>,
    state_channel: ImmediatePublisher<'a, M, AnyState, 2, 6, 0>,
    lock_pin: L,
    reed_pin: R,
    last_reed_state: PinState,
}

impl<'a, L, R, M> Door<'a, L, R, M>
where
    L: OutputPin + StatefulOutputPin,
    R: InputPin + Wait,
    M: RawMutex,
{
    pub fn new(
        lock_pin: L,
        reed_pin: R,
        cmd_channel: Receiver<'a, M, LockState, 2>,
        state_channel: ImmediatePublisher<'a, M, AnyState, 2, 6, 0>,
    ) -> Self {
        Self {
            lock_pin: lock_pin,
            reed_pin: reed_pin,
            cmd_channel: cmd_channel,
            state_channel: state_channel,
            last_reed_state: PinState::Low,
        }
    }

    pub async fn run(&mut self) {
        if let Ok(true) = self.reed_pin.is_high() {
            self.last_reed_state = PinState::High;
        }

        loop {
            let work = select::select(
                self.cmd_channel.receive(),
                self.reed_pin.wait_for_any_edge(),
            )
            .await;

            match work {
                select::Either::First(LockState::Locked) => {
                    info!("received lock command");
                    if let Err(e) = self.lock().await {
                        error!("error locking door: {}", e.kind());
                    }
                }
                select::Either::First(LockState::Unlocked) => {
                    info!("received unlock command");
                    if let Err(e) = self.unlock().await {
                        error!("error unlocking door: {}", e.kind());
                    }
                }
                select::Either::Second(Ok(())) => {
                    // The door is closed when the reed is "ON" and grounding the pin.
                    match self.reed_pin.is_low() {
                        Ok(result) => {
                            if result {
                                if self.last_reed_state == PinState::High {
                                    // Low to High transition
                                    info!("door is closed");
                                    self.state_channel
                                        .publish_immediate(AnyState::DoorState(DoorState::Closed));
                                }
                                self.last_reed_state = PinState::Low;
                            } else {
                                if self.last_reed_state == PinState::Low {
                                    // High to Low transition
                                    info!("door is Open");
                                    self.state_channel
                                        .publish_immediate(AnyState::DoorState(DoorState::Open));
                                }
                                self.last_reed_state = PinState::High;
                            }
                        }
                        Err(e) => error!("error reading reed state: {}", e.kind()),
                    };
                }
                select::Either::Second(Err(e)) => {
                    error!("error waiting for reed pin: {}", e.kind());
                }
            }
        }
    }

    pub async fn lock(&mut self) -> Result<(), <L as ErrorType>::Error> {
        self.lock_pin.set_low()?;
        self.state_channel
            .publish_immediate(AnyState::LockState(LockState::Locked));

        Ok(())
    }

    pub async fn unlock(&mut self) -> Result<(), <L as ErrorType>::Error> {
        self.lock_pin.set_high()?;
        self.state_channel
            .publish_immediate(AnyState::LockState(LockState::Unlocked));

        Ok(())
    }
}
