#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_rp::{
    bind_interrupts, gpio,
    peripherals::USB,
    spi,
    usb::{Driver, InterruptHandler},
};
use embassy_time::Timer;
use embassy_usb::{
    Builder, Config,
    class::cdc_acm::{CdcAcmClass, State},
};

use panic_halt as _;

mod usb;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut spi_config = spi::Config::default();
    spi_config.frequency = 256_000;
    spi_config.phase = spi::Phase::CaptureOnSecondTransition;
    spi_config.polarity = spi::Polarity::IdleHigh;

    let mut spi = spi::Spi::new_blocking(p.SPI0, p.PIN_2, p.PIN_3, p.PIN_4, spi_config);

    let driver = Driver::new(p.USB, Irqs);
    // Shared development VID/PID; identify this device by its product and INFO reply.
    // The host must confirm the PMB3 INFO response, not rely on VID/PID alone.
    let mut config = Config::new(0x16c0, 0x27dd);
    config.manufacturer = Some("OpenBoy Project");
    config.product = Some("Pico GBA Multibooter");
    config.max_power = 100;
    config.max_packet_size_0 = 64;
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buffer = [0; 64];
    let mut state = State::new();
    let bus = usb::BusState::default();
    let mut handler = usb::BusHandler(&bus);
    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [],
        &mut control_buffer,
    );
    builder.handler(&mut handler);
    let class = CdcAcmClass::new(&mut builder, &mut state, 64);
    let mut device = builder.build();

    let mut led = gpio::Output::new(p.PIN_25, gpio::Level::Low);
    let heartbeat = async {
        loop {
            Timer::after_secs(1).await;
            led.toggle();
        }
    };
    join3(device.run(), usb::serve(class, &mut spi, &bus), heartbeat).await;
}
