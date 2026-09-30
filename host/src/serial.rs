use crate::Error;
use serialport::{ClearBuffer, SerialPort, SerialPortType};
use std::{
    io::{self, Read, Write},
    time::Duration,
};

const PRODUCT: &str = "Pico GBA Multibooter";

/// Blocking byte transport. Implementations must honor the timeout for both
/// reads and writes, returning TimedOut/WouldBlock when no progress is possible.
pub trait Transport: Read + Write {
    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()>;
    fn set_dtr(&mut self, asserted: bool) -> io::Result<()>;
    fn clear_input(&mut self) -> io::Result<()>;
}

pub struct SerialTransport {
    port: Box<dyn SerialPort>,
}

impl SerialTransport {
    pub fn open(name: &str) -> Result<Self, Error> {
        // CDC baud is metadata, not the USB transfer rate. Serialport opens
        // exclusively on supported Unix/Windows platforms.
        let port = serialport::new(name, 115_200)
            .timeout(Duration::from_millis(100))
            .flow_control(serialport::FlowControl::None)
            .dtr_on_open(false)
            .open()
            .map_err(|e| Error::io("open serial port", io::Error::from(e)))?;
        Ok(Self { port })
    }
}

impl Read for SerialTransport {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.port.read(bytes)
    }
}
impl Write for SerialTransport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.port.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.port.flush()
    }
}
impl Transport for SerialTransport {
    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.port.set_timeout(timeout).map_err(Into::into)
    }
    fn set_dtr(&mut self, asserted: bool) -> io::Result<()> {
        self.port
            .write_data_terminal_ready(asserted)
            .map_err(Into::into)
    }
    fn clear_input(&mut self) -> io::Result<()> {
        self.port.clear(ClearBuffer::Input).map_err(Into::into)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortInfo {
    pub name: String,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub usb_id: Option<(u16, u16)>,
}

impl PortInfo {
    pub fn is_candidate(&self) -> bool {
        self.product.as_deref() == Some(PRODUCT)
    }
}

/// Enumerate without opening ports. Unless `all` is set, return only devices
/// whose product name matches Pico GBA Multibooter. INFO is checked on open.
pub fn list_ports(all: bool) -> Result<Vec<PortInfo>, Error> {
    let ports = serialport::available_ports()
        .map_err(|e| Error::io("list serial ports", io::Error::from(e)))?;
    let ports: Vec<_> = ports
        .into_iter()
        .map(|port| {
            let (product, serial_number, usb_id) = match port.port_type {
                SerialPortType::UsbPort(usb) => {
                    (usb.product, usb.serial_number, Some((usb.vid, usb.pid)))
                }
                _ => (None, None, None),
            };
            PortInfo {
                name: port.port_name,
                product,
                serial_number,
                usb_id,
            }
        })
        .collect();
    Ok(filter_ports(ports, all))
}

pub(crate) fn filter_ports(mut ports: Vec<PortInfo>, all: bool) -> Vec<PortInfo> {
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    if !all {
        ports.retain(PortInfo::is_candidate);
    }
    ports
}

/// Select only one product-name match. The shared development VID/PID alone
/// is insufficient. Opening the selected port still verifies PMB3 with INFO.
pub fn select_port(ports: &[PortInfo]) -> Result<String, Error> {
    let candidates: Vec<_> = ports
        .iter()
        .filter(|p| p.is_candidate())
        .map(|p| p.name.clone())
        .collect();
    match candidates.len() {
        0 => Err(Error::NoCable),
        1 => Ok(candidates.into_iter().next().unwrap()),
        _ => Err(Error::AmbiguousPorts(candidates)),
    }
}
