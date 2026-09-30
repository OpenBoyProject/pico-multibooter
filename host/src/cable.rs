use crate::{Error, SerialTransport, Transport};
use protocol::{Command, Decoder, DeviceInfo, MAX_BULK_WORDS, MAX_FRAME, Request, Response, Words};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const IO_SLICE: Duration = Duration::from_millis(100);

/// Clone this handle into a signal handler or another thread to cancel I/O.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct Options {
    /// Deadline for one USB request/reply transaction. Default: 3 seconds.
    pub command_timeout: Duration,
    /// Wait after lowering and raising DTR. Default: 150 ms each.
    pub reset_delay: Duration,
    pub cancellation: Cancellation,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_secs(3),
            reset_delay: Duration::from_millis(150),
            cancellation: Cancellation::default(),
        }
    }
}

enum Reply {
    Info(DeviceInfo),
    Written,
    Word(u32),
    Words(Vec<u32>),
}

/// Exclusive, sequential connection. Any transaction error closes the session
/// and lowers DTR; a new connection is required before issuing another request.
pub struct Cable<T: Transport = SerialTransport> {
    transport: T,
    options: Options,
    usable: bool,
    bulk_capacity: usize,
}

impl Cable<SerialTransport> {
    pub fn open(port: &str) -> Result<Self, Error> {
        Self::open_with_options(port, Options::default())
    }
    pub fn open_with_options(port: &str, options: Options) -> Result<Self, Error> {
        Self::connect(SerialTransport::open(port)?, options)
    }
}

impl<T: Transport> Cable<T> {
    /// Establish a fresh session: lower DTR, discard stale input, raise DTR,
    /// then verify the firmware identity. No GBA words are clocked here.
    pub fn connect(transport: T, options: Options) -> Result<Self, Error> {
        let now = Instant::now();
        if options.command_timeout.is_zero() || now.checked_add(options.command_timeout).is_none() {
            return Err(Error::InvalidTimeout);
        }
        if now.checked_add(options.reset_delay).is_none() {
            return Err(Error::InvalidTimeout);
        }
        let mut cable = Self {
            transport,
            options,
            usable: true,
            bulk_capacity: 0,
        };
        cable
            .transport
            .set_dtr(false)
            .map_err(|e| Error::io("lower DTR", e))?;
        cable.pause(cable.options.reset_delay)?;
        cable
            .transport
            .clear_input()
            .map_err(|e| Error::io("clear serial input", e))?;
        cable
            .transport
            .set_dtr(true)
            .map_err(|e| Error::io("raise DTR", e))?;
        cable.pause(cable.options.reset_delay)?;
        cable
            .transport
            .clear_input()
            .map_err(|e| Error::io("clear serial input", e))?;
        cable.info()?;
        Ok(cable)
    }

    pub fn is_usable(&self) -> bool {
        self.usable
    }

    pub fn close(&mut self) -> Result<(), Error> {
        self.usable = false;
        self.transport
            .set_dtr(false)
            .map_err(|e| Error::io("lower DTR", e))
    }

    /// Query cable identity and limits without clocking the GBA. Refreshes the cache.
    pub fn info(&mut self) -> Result<DeviceInfo, Error> {
        match self.request(Request::Info)? {
            Reply::Info(info) => {
                self.bulk_capacity = info.bulk_capacity as usize;
                Ok(info)
            }
            _ => unreachable!("typed protocol checked the response"),
        }
    }

    /// Exchange exactly one word.
    pub fn exchange(&mut self, word: u32) -> Result<u32, Error> {
        match self.request(Request::Exchange(word))? {
            Reply::Word(word) => Ok(word),
            _ => unreachable!("typed protocol checked the response"),
        }
    }

    /// Bulk word limit cached from INFO during connection. Performs no I/O.
    pub fn bulk_capacity(&self) -> Result<usize, Error> {
        if !self.usable {
            return Err(Error::SessionLost);
        }
        Ok(self.bulk_capacity)
    }

    /// Full-duplex bulk transaction. No automatic batching or retry; the caller
    /// defines word meanings and accounts for its GBA program's response latency.
    pub fn bulk_exchange(&mut self, words: &[u32]) -> Result<Vec<u32>, Error> {
        self.check_bulk_count(words.len())?;
        let mut bytes = [0; MAX_BULK_WORDS * 4];
        let words = Words::from_words(words, &mut bytes).unwrap();
        match self.request(Request::BulkExchange { words })? {
            Reply::Words(words) => Ok(words),
            _ => unreachable!("typed protocol checked the response"),
        }
    }

    /// Receive words while the Pico transmits `fill` for every SPI word. Only
    /// the count and fill value cross USB in the request, not an array of fills.
    pub fn bulk_read(&mut self, count: usize, fill: u32) -> Result<Vec<u32>, Error> {
        self.check_bulk_count(count)?;
        match self.request(Request::BulkRead {
            count: count as u16,
            fill,
        })? {
            Reply::Words(words) => Ok(words),
            _ => unreachable!("typed protocol checked the response"),
        }
    }

    /// Write raw application words, discarding received SPI words. The reply
    /// confirms that all words were clocked, not that the GBA accepted them.
    /// Multiboot ROM encoding is handled by the separate `mb-uploader` crate.
    pub fn bulk_write(&mut self, words: &[u32]) -> Result<(), Error> {
        self.check_bulk_count(words.len())?;
        let mut bytes = [0; MAX_BULK_WORDS * 4];
        let words = Words::from_words(words, &mut bytes).unwrap();
        self.request(Request::BulkWrite { words })?;
        Ok(())
    }

    fn check_bulk_count(&mut self, count: usize) -> Result<(), Error> {
        check_word_count(count)?;
        let maximum = self.bulk_capacity()?;
        if count > maximum {
            return Err(Error::BulkCapacity {
                requested: count,
                maximum,
            });
        }
        Ok(())
    }

    fn request(&mut self, request: Request<'_>) -> Result<Reply, Error> {
        if !self.usable {
            return Err(Error::SessionLost);
        }
        let command = request.command();
        let mut output = [0; MAX_FRAME];
        let length = request
            .encode(&mut output)
            .map_err(|source| Error::Protocol { command, source })?;
        let deadline = Instant::now()
            .checked_add(self.options.command_timeout)
            .ok_or(Error::InvalidTimeout)?;
        let result = self.transact(request, &output[..length], deadline);
        if result.is_err() {
            let _ = self.close();
        }
        result
    }

    fn transact(
        &mut self,
        request: Request<'_>,
        output: &[u8],
        deadline: Instant,
    ) -> Result<Reply, Error> {
        let command = request.command();
        let mut sent = 0;
        while sent < output.len() {
            self.prepare_io(command, deadline)?;
            match self.transport.write(&output[sent..]) {
                Ok(0) => return Err(Error::io("write request", io::ErrorKind::WriteZero)),
                Ok(count) => sent += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // An errored write may have an unknown effect; do not repeat it.
                Err(error) => return Err(Error::io("write request", error)),
            }
        }
        // Do not call a potentially unbounded OS drain/flush. The reply confirms
        // delivery; continue polling for that reply, never resending the request.
        let mut decoder = Decoder::new();
        let mut input = [0; 64];
        loop {
            self.prepare_io(command, deadline)?;
            let count = match self.transport.read(&mut input) {
                Ok(0) => return Err(Error::io("read reply", io::ErrorKind::UnexpectedEof)),
                Ok(count) => count,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(Error::io("read reply", error)),
            };
            for (index, &byte) in input[..count].iter().enumerate() {
                if let Some(frame) = decoder.push(byte) {
                    let frame = frame.map_err(|error| Error::Protocol {
                        command,
                        source: error.error,
                    })?;
                    let response = Response::decode_for(request, frame)
                        .map_err(|source| Error::Protocol { command, source })?;
                    if index + 1 != count {
                        return Err(Error::Protocol {
                            command,
                            source: protocol::Error::InvalidPayload,
                        });
                    }
                    // Also observe cancellation/deadline after the final read.
                    self.check_deadline(command, deadline)?;
                    return match response {
                        Response::Info(info) => Ok(Reply::Info(info)),
                        Response::Written(_) => Ok(Reply::Written),
                        Response::Exchange(word) => Ok(Reply::Word(word)),
                        Response::BulkData(words) => Ok(Reply::Words(words.iter().collect())),
                        Response::Error(code) => Err(Error::Device { command, code }),
                    };
                }
            }
        }
    }

    fn check_deadline(&self, command: Command, deadline: Instant) -> Result<(), Error> {
        if self.options.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout { command });
        }
        Ok(())
    }

    fn prepare_io(&mut self, command: Command, deadline: Instant) -> Result<(), Error> {
        self.check_deadline(command, deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::Timeout { command });
        }
        self.transport
            .set_timeout(remaining.min(IO_SLICE))
            .map_err(|e| Error::io("set serial timeout", e))
    }

    /// Wait locally without clocking the device. Cancellation closes the session.
    pub fn wait(&mut self, duration: Duration) -> Result<(), Error> {
        if !self.usable {
            return Err(Error::SessionLost);
        }
        let result = self.pause(duration);
        if result.is_err() {
            let _ = self.close();
        }
        result
    }

    fn pause(&self, duration: Duration) -> Result<(), Error> {
        let until = Instant::now()
            .checked_add(duration)
            .ok_or(Error::InvalidTimeout)?;
        loop {
            if self.options.cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }
}

impl<T: Transport> Drop for Cable<T> {
    fn drop(&mut self) {
        let _ = self.transport.set_dtr(false);
    }
}

fn check_word_count(count: usize) -> Result<(), Error> {
    if !(1..=MAX_BULK_WORDS).contains(&count) {
        return Err(Error::InvalidWordCount(count));
    }
    Ok(())
}
