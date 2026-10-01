use core::cell::Cell;

use embassy_futures::select::{Either, select};
use embassy_rp::{
    gpio::Output,
    peripherals::{SPI0, USB},
    spi::{Blocking, Spi},
    usb::Driver,
};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embassy_usb::{
    Handler,
    class::cdc_acm::{CdcAcmClass, ControlChanged, Receiver, Sender},
    control::{OutResponse, Recipient, Request, RequestType},
    driver::EndpointError,
};
use firmware::{
    link::{Link, WORD_DELAY_US},
    server,
};
use protocol::{Decoder, ErrorCode, MAX_FRAME};

const PACKET_SIZE: usize = 64;
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

struct Activity<'a, 'd>(&'a mut Output<'d>);

impl Drop for Activity<'_, '_> {
    fn drop(&mut self) {
        // Also runs when a disconnect cancels the transaction future.
        self.0.set_low();
    }
}

/// Accessed only by futures on the same executor thread.
#[derive(Default)]
pub struct BusState {
    configured: Cell<bool>,
    suspended: Cell<bool>,
    generation: Cell<u32>,
}

impl BusState {
    fn invalidate(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
    }
    fn ready(&self) -> bool {
        self.configured.get() && !self.suspended.get()
    }
}

pub struct BusHandler<'a>(pub &'a BusState);

impl Handler for BusHandler<'_> {
    fn enabled(&mut self, enabled: bool) {
        if !enabled {
            self.0.configured.set(false);
            self.0.invalidate();
        }
    }
    fn reset(&mut self) {
        self.0.configured.set(false);
        self.0.suspended.set(false);
        self.0.invalidate();
    }
    fn configured(&mut self, configured: bool) {
        self.0.configured.set(configured);
        self.0.invalidate();
    }
    fn suspended(&mut self, suspended: bool) {
        self.0.suspended.set(suspended);
        if suspended {
            self.0.invalidate();
        }
    }
    fn control_out(&mut self, request: Request, _data: &[u8]) -> Option<OutResponse> {
        // Registered before CDC. Observe DTR drops on its control interface (0)
        // without consuming the request. The generation remembers even a rapid
        // drop/reassertion that occurs between application polls.
        if request.request_type == RequestType::Class
            && request.recipient == Recipient::Interface
            && request.index == 0
            && request.request == 0x22
            && request.value & 1 == 0
        {
            self.0.invalidate();
        }
        None
    }
}

#[derive(Clone, Copy)]
struct Session<'a, 'd> {
    bus: &'a BusState,
    control: &'a ControlChanged<'d>,
    generation: u32,
}

impl Session<'_, '_> {
    fn active(self) -> bool {
        self.bus.ready() && self.control.dtr() && self.bus.generation.get() == self.generation
    }

    async fn wait_end(self) {
        while self.active() {
            // Control changes wake DTR cancellation immediately. The timer also
            // observes bus events which need not signal CDC control changes.
            select(self.control.control_changed(), Timer::after_millis(1)).await;
        }
    }
}

struct SpiLink<'a, 'd, F> {
    spi: &'a mut Spi<'d, SPI0, Blocking>,
    connected: F,
}

impl<F: Fn() -> bool> Link for SpiLink<'_, '_, F> {
    async fn exchange(&mut self, word: u32) -> Result<u32, ErrorCode> {
        if !(self.connected)() {
            return Err(ErrorCode::Disconnected);
        }
        let mut bytes = word.to_be_bytes();
        self.spi
            .blocking_transfer_in_place(&mut bytes)
            .map_err(|_| ErrorCode::TransferFailed)?;
        Timer::after_micros(WORD_DELAY_US).await;
        if !(self.connected)() {
            return Err(ErrorCode::Disconnected);
        }
        Ok(u32::from_be_bytes(bytes))
    }
}

pub async fn serve<'d>(
    class: CdcAcmClass<'d, Driver<'d, USB>>,
    spi: &mut Spi<'_, SPI0, Blocking>,
    bus: &BusState,
    led: &mut Output<'_>,
) -> ! {
    let (mut tx, mut rx, control) = class.split_with_control();
    loop {
        rx.wait_connection().await;
        // Drop incoming data while the port is closed, rather than carrying a
        // previous session's queued bytes into a newly asserted DTR session.
        let mut discarded = [0; PACKET_SIZE];
        while !bus.ready() || !control.dtr() {
            select(
                rx.read_packet(&mut discarded),
                select(control.control_changed(), Timer::after_millis(10)),
            )
            .await;
            if !bus.ready() {
                Timer::after_millis(10).await;
            }
        }
        let session = Session {
            bus,
            control: &control,
            generation: bus.generation.get(),
        };
        let mut link = SpiLink {
            spi,
            connected: || session.active(),
        };
        // Cancelling drops the decoder and any pending transfer/reply.
        select(
            session.wait_end(),
            run_session(&mut tx, &mut rx, &mut link, led),
        )
        .await;
    }
}

async fn run_session<'d>(
    tx: &mut Sender<'d, Driver<'d, USB>>,
    rx: &mut Receiver<'d, Driver<'d, USB>>,
    link: &mut impl Link,
    led: &mut Output<'_>,
) {
    let mut decoder = Decoder::new();
    let mut input = [0; PACKET_SIZE];
    let mut output = [0; MAX_FRAME];
    let mut deadline = Instant::now() + IDLE_TIMEOUT;
    loop {
        let count = match select(Timer::at(deadline), rx.read_packet(&mut input)).await {
            Either::Second(Ok(count)) => count,
            _ => return,
        };
        if count == 0 {
            continue;
        }
        for &byte in &input[..count] {
            let Some(frame) = decoder.push(byte) else {
                continue;
            };
            led.set_high();
            let _activity = Activity(led);
            let result = match frame {
                Ok(frame) => server::handle(frame, link, &mut output).await,
                Err(error) => server::reject(error.command, ErrorCode::InvalidFrame, &mut output),
            };
            let Ok(length) = result else {
                return;
            };
            if !matches!(
                with_timeout(REPLY_TIMEOUT, write_reply(tx, &output[..length])).await,
                Ok(Ok(()))
            ) {
                return;
            }
        }
        deadline = Instant::now() + IDLE_TIMEOUT;
    }
}

async fn write_reply<'d>(
    tx: &mut Sender<'d, Driver<'d, USB>>,
    bytes: &[u8],
) -> Result<(), EndpointError> {
    for packet in bytes.chunks(PACKET_SIZE) {
        tx.write_packet(packet).await?;
    }
    // End an exact-multiple bulk transfer with a short packet so a host read
    // waiting for more data does not retain the last full packet indefinitely.
    if bytes.len().is_multiple_of(PACKET_SIZE) {
        tx.write_packet(&[]).await?;
    }
    Ok(())
}
