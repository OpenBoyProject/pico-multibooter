use crate::{Cable, Cancellation, Error, Options, PortInfo, Transport, select_port};
use protocol::{Command, Decoder, ErrorCode, Frame, MAX_FRAME};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[derive(Clone, Copy)]
enum Fault {
    LostReply,
    CorruptCrc,
    WrongCommand,
    WrongOffset,
    DeviceError,
    Oversized,
}

struct State {
    decoder: Decoder,
    replies: VecDeque<u8>,
    requests: Vec<(u8, Vec<u8>)>,
    dtr: Vec<bool>,
    clears: usize,
    timeout: Duration,
    fault: Option<(u8, Fault)>,
    max_read: usize,
    max_write: usize,
    write_error_after: Option<usize>,
    written: usize,
    write_zero: bool,
    read_eof: bool,
    noise: bool,
    capacity: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            decoder: Decoder::new(),
            replies: VecDeque::new(),
            requests: Vec::new(),
            dtr: Vec::new(),
            clears: 0,
            timeout: Duration::ZERO,
            fault: None,
            max_read: 2,
            max_write: 3,
            write_error_after: None,
            written: 0,
            write_zero: false,
            read_eof: false,
            noise: false,
            capacity: 256,
        }
    }
}

impl State {
    fn reply(&mut self, command: u8, request: Vec<u8>) {
        let mut payload = vec![0];
        match command {
            8 => {
                payload.extend_from_slice(b"PICO-MB3");
                payload.extend_from_slice(&(self.capacity as u32).to_le_bytes());
            }
            6 => {
                payload.extend_from_slice(&(request.len() as u32).to_le_bytes());
            }
            3 | 7 => {
                for word in request.as_chunks::<4>().0 {
                    let word = u32::from_le_bytes(*word).wrapping_add(0x100);
                    payload.extend_from_slice(&word.to_le_bytes());
                }
            }
            5 => {
                let count = u16::from_le_bytes(request[..2].try_into().unwrap());
                let fill = u32::from_le_bytes(request[2..6].try_into().unwrap());
                for _ in 0..count {
                    payload.extend_from_slice(&fill.wrapping_add(0x100).to_le_bytes());
                }
            }
            _ => panic!("unexpected command {command}"),
        }
        self.requests.push((command, request));
        let fault = self
            .fault
            .filter(|(cmd, _)| *cmd == command)
            .map(|(_, fault)| fault);
        let mut reply_command = command | 0x80;
        match fault {
            Some(Fault::LostReply) => return,
            Some(Fault::WrongCommand) => reply_command = 0xff,
            Some(Fault::WrongOffset) => payload = vec![0; 5],
            Some(Fault::DeviceError) => payload = vec![ErrorCode::TransferFailed as u8],
            Some(Fault::Oversized) => {
                self.replies
                    .extend([b'P', b'M', b'B', b'3', reply_command, 0xff, 0xff]);
                return;
            }
            _ => {}
        }
        let mut bytes = [0; MAX_FRAME];
        let len = Frame {
            command: reply_command,
            payload: &payload,
        }
        .encode(&mut bytes)
        .unwrap();
        if matches!(fault, Some(Fault::CorruptCrc)) {
            bytes[len - 1] ^= 0x80;
        }
        if self.noise {
            self.replies.extend(b"noisePPM".iter().copied());
        }
        self.replies.extend(&bytes[..len]);
    }
}

struct Mock(Arc<Mutex<State>>);

impl Read for Mock {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut state = self.0.lock().unwrap();
        if state.read_eof {
            return Ok(0);
        }
        if state.replies.is_empty() {
            let timeout = state.timeout;
            drop(state);
            thread::sleep(timeout);
            return Err(io::ErrorKind::TimedOut.into());
        }
        let count = buffer.len().min(state.max_read).min(state.replies.len());
        for byte in &mut buffer[..count] {
            *byte = state.replies.pop_front().unwrap();
        }
        Ok(count)
    }
}

impl Write for Mock {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut state = self.0.lock().unwrap();
        if state.write_zero {
            return Ok(0);
        }
        if state
            .write_error_after
            .is_some_and(|limit| state.written >= limit)
        {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let count = buffer.len().min(state.max_write);
        state.written += count;
        for &byte in &buffer[..count] {
            let decoded = state.decoder.push(byte).map(|result| {
                let frame = result.unwrap();
                (frame.command, frame.payload.to_vec())
            });
            if let Some((command, payload)) = decoded {
                state.reply(command, payload);
            }
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("client must not call an unbounded OS flush")
    }
}

impl Transport for Mock {
    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        assert!(!timeout.is_zero());
        assert!(timeout <= Duration::from_millis(100));
        self.0.lock().unwrap().timeout = timeout;
        Ok(())
    }
    fn set_dtr(&mut self, asserted: bool) -> io::Result<()> {
        self.0.lock().unwrap().dtr.push(asserted);
        Ok(())
    }
    fn clear_input(&mut self) -> io::Result<()> {
        let mut state = self.0.lock().unwrap();
        state.clears += 1;
        state.replies.clear();
        Ok(())
    }
}

fn options() -> Options {
    Options {
        reset_delay: Duration::ZERO,
        ..Options::default()
    }
}
fn connect(state: Arc<Mutex<State>>) -> Cable<Mock> {
    Cable::connect(Mock(state), options()).unwrap()
}

#[test]
fn bad_crc_wrong_command_bad_count_and_oversize_close_session() {
    for fault in [
        Fault::CorruptCrc,
        Fault::WrongCommand,
        Fault::WrongOffset,
        Fault::Oversized,
    ] {
        let state = Arc::new(Mutex::new(State {
            fault: Some((6, fault)),
            ..State::default()
        }));
        let mut cable = connect(state.clone());
        let error = cable.bulk_write(&[0; 256]).unwrap_err();
        assert!(matches!(
            error,
            Error::Protocol {
                command: Command::BulkWrite,
                ..
            }
        ));
        assert!(!cable.is_usable());
        assert!(matches!(cable.info(), Err(Error::SessionLost)));
        let state = state.lock().unwrap();
        assert_eq!(
            state.requests.iter().map(|r| r.0).collect::<Vec<_>>(),
            [8, 6]
        );
        assert!(!state.dtr.last().unwrap());
    }
}

#[test]
fn partial_write_failure_zero_write_and_eof_poison_the_connection() {
    for mode in 0..3 {
        let state = Arc::new(Mutex::new(State::default()));
        let mut cable = connect(state.clone());
        {
            let mut state = state.lock().unwrap();
            match mode {
                0 => state.write_error_after = Some(state.written + 3),
                1 => state.write_zero = true,
                _ => state.read_eof = true,
            }
        }
        assert!(matches!(cable.info(), Err(Error::Io { .. })));
        assert!(!cable.is_usable());
        let written = state.lock().unwrap().written;
        assert!(matches!(cable.info(), Err(Error::SessionLost)));
        assert_eq!(state.lock().unwrap().written, written);
    }
}

#[test]
fn cancellation_prevents_new_words_and_interrupts_local_waits() {
    for wait in [false, true] {
        let state = Arc::new(Mutex::new(State::default()));
        let cancellation = Cancellation::default();
        let mut cable = Cable::connect(
            Mock(state.clone()),
            Options {
                cancellation: cancellation.clone(),
                ..options()
            },
        )
        .unwrap();
        cancellation.cancel();
        let result = if wait {
            cable.wait(Duration::from_secs(1))
        } else {
            cable.exchange(0).map(|_| ())
        };
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(!cable.is_usable());
        let state = state.lock().unwrap();
        assert_eq!(state.requests.iter().map(|r| r.0).collect::<Vec<_>>(), [8]);
        assert!(!state.dtr.last().unwrap());
    }
}

#[test]
fn connection_identity_failure_also_lowers_dtr() {
    let state = Arc::new(Mutex::new(State {
        fault: Some((8, Fault::WrongCommand)),
        ..State::default()
    }));
    assert!(matches!(
        Cable::connect(Mock(state.clone()), options()),
        Err(Error::Protocol { .. })
    ));
    assert!(!state.lock().unwrap().dtr.last().unwrap());
}

#[test]
fn device_errors_close_session() {
    let state = Arc::new(Mutex::new(State {
        fault: Some((3, Fault::DeviceError)),
        ..State::default()
    }));
    let mut cable = connect(state);
    let error = cable.exchange(0).unwrap_err();
    assert!(matches!(
        error,
        Error::Device {
            command: Command::Exchange,
            code: ErrorCode::TransferFailed
        }
    ));
    assert!(!cable.is_usable());
}

#[test]
fn exchange_checks_capability_and_preserves_word_order() {
    let state = Arc::new(Mutex::new(State {
        noise: true,
        ..State::default()
    }));
    let mut cable = connect(state.clone());
    assert_eq!(
        cable.bulk_exchange(&[0x1234_5678, u32::MAX]).unwrap(),
        [0x1234_5778, 0xff]
    );
    assert_eq!(cable.bulk_read(2, 0).unwrap(), [0x100, 0x100]);
    assert_eq!(cable.bulk_exchange(&[42; 256]).unwrap(), [0x12a; 256]);
    assert!(matches!(
        cable.bulk_exchange(&[]),
        Err(Error::InvalidWordCount(0))
    ));
    assert!(matches!(
        cable.bulk_read(257, 0),
        Err(Error::InvalidWordCount(257))
    ));
    let state = state.lock().unwrap();
    assert_eq!(state.requests.iter().filter(|r| r.0 == 8).count(), 1);
    assert_eq!(
        state
            .requests
            .iter()
            .filter(|r| matches!(r.0, 5 | 7))
            .count(),
        3
    );
}

#[test]
fn limited_capability_does_not_clock_an_oversized_exchange() {
    let state = Arc::new(Mutex::new(State {
        capacity: 1,
        ..State::default()
    }));
    let mut cable = connect(state.clone());
    assert!(matches!(
        cable.bulk_exchange(&[1, 2]),
        Err(Error::BulkCapacity {
            requested: 2,
            maximum: 1
        })
    ));
    assert!(cable.is_usable());
    assert_eq!(
        state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.0 == 8)
            .count(),
        1
    );
}

#[test]
fn lost_exchange_reply_is_not_repeated() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut opts = options();
    opts.command_timeout = Duration::from_millis(10);
    let mut cable = Cable::connect(Mock(state.clone()), opts).unwrap();
    cable.bulk_capacity().unwrap();
    state.lock().unwrap().fault = Some((7, Fault::LostReply));
    assert!(matches!(
        cable.bulk_exchange(&[1]),
        Err(Error::Timeout {
            command: Command::BulkExchange
        })
    ));
    assert!(matches!(cable.bulk_exchange(&[1]), Err(Error::SessionLost)));
    assert_eq!(
        state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| matches!(r.0, 5 | 7))
            .count(),
        1
    );
}

#[test]
fn port_selection_requires_one_product_match_not_just_shared_vid_pid() {
    let unrelated = PortInfo {
        name: "COM1".into(),
        product: Some("Other device".into()),
        serial_number: None,
        usb_id: Some((0x16c0, 0x27dd)),
    };
    assert!(matches!(
        select_port(std::slice::from_ref(&unrelated)),
        Err(Error::NoCable)
    ));
    let pico = PortInfo {
        name: "COM3".into(),
        product: Some("Pico GBA Multibooter".into()),
        ..unrelated.clone()
    };
    assert_eq!(
        select_port(&[unrelated.clone(), pico.clone()]).unwrap(),
        "COM3"
    );
    let other = PortInfo {
        name: "COM4".into(),
        ..pico.clone()
    };
    assert_eq!(select_port(std::slice::from_ref(&other)).unwrap(), "COM4");
    for product in [
        None,
        Some("Pico GBA Multiboot".into()),
        Some("Pico GBA Multibooter Clone".into()),
    ] {
        let port = PortInfo {
            product,
            ..unrelated.clone()
        };
        assert!(matches!(select_port(&[port]), Err(Error::NoCable)));
    }
    assert!(
        matches!(select_port(&[pico, other]), Err(Error::AmbiguousPorts(ports)) if ports == ["COM3", "COM4"])
    );
}

#[test]
fn invalid_timeouts_are_rejected_before_session_setup() {
    let state = Arc::new(Mutex::new(State::default()));
    let options = Options {
        command_timeout: Duration::ZERO,
        ..options()
    };
    assert!(matches!(
        Cable::connect(Mock(state.clone()), options),
        Err(Error::InvalidTimeout)
    ));
    assert!(state.lock().unwrap().dtr.is_empty());
}

#[test]
fn bulk_exchanges_use_cached_capacity_and_never_fall_back() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut cable = connect(state.clone());
    assert_eq!(cable.bulk_exchange(&[42; 256]).unwrap(), [0x12a; 256]);
    assert_eq!(cable.bulk_exchange(&[1]).unwrap(), [0x101]);
    cable.bulk_exchange(&[2]).unwrap();
    {
        let state = state.lock().unwrap();
        assert_eq!(
            state.requests.iter().map(|r| r.0).collect::<Vec<_>>(),
            [8, 7, 7, 7]
        );
    }
    let state = Arc::new(Mutex::new(State {
        fault: Some((7, Fault::DeviceError)),
        ..State::default()
    }));
    let mut cable = connect(state.clone());
    assert!(cable.bulk_exchange(&[1]).is_err());
    assert!(!cable.is_usable());
    assert_eq!(
        state
            .lock()
            .unwrap()
            .requests
            .iter()
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        [8, 7]
    );
}

#[test]
fn lost_bulk_exchange_reply_is_not_repeated() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut opts = options();
    opts.command_timeout = Duration::from_millis(10);
    let mut cable = Cable::connect(Mock(state.clone()), opts).unwrap();
    cable.bulk_capacity().unwrap();
    state.lock().unwrap().fault = Some((7, Fault::LostReply));
    assert!(matches!(
        cable.bulk_exchange(&[1]),
        Err(Error::Timeout {
            command: Command::BulkExchange
        })
    ));
    assert!(!cable.is_usable());
    assert_eq!(
        state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.0 == 7)
            .count(),
        1
    );
}

#[test]
fn single_word_read_and_write_use_distinct_commands_and_compact_payloads() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut cable = connect(state.clone());
    assert_eq!(cable.exchange(0x12345678).unwrap(), 0x12345778);

    assert_eq!(
        cable.bulk_read(256, 0x12345678).unwrap(),
        vec![0x12345778; 256]
    );
    cable.bulk_write(&[42; 256]).unwrap();

    let state = state.lock().unwrap();
    assert_eq!(
        state.requests.iter().map(|r| r.0).collect::<Vec<_>>(),
        [8, 3, 5, 6]
    );
    assert_eq!(state.requests[1].1, 0x12345678u32.to_le_bytes());
    assert_eq!(state.requests[2].1, [0, 1, 0x78, 0x56, 0x34, 0x12]);
    assert_eq!(state.requests[3].1, 42u32.to_le_bytes().repeat(256));
}

#[test]
fn lost_read_write_or_single_exchange_reply_is_never_replayed() {
    for command in [6, 3, 5] {
        let state = Arc::new(Mutex::new(State::default()));
        let mut opts = options();
        opts.command_timeout = Duration::from_millis(10);
        let mut cable = Cable::connect(Mock(state.clone()), opts).unwrap();
        state.lock().unwrap().fault = Some((command, Fault::LostReply));
        let result = match command {
            6 => cable.bulk_write(&[1, 2]),
            3 => cable.exchange(1).map(|_| ()),
            5 => cable.bulk_read(2, 0).map(|_| ()),
            _ => unreachable!(),
        };
        assert!(matches!(result, Err(Error::Timeout { .. })));
        assert!(!cable.is_usable());
        assert_eq!(
            state
                .lock()
                .unwrap()
                .requests
                .iter()
                .filter(|r| r.0 == command)
                .count(),
            1
        );
        assert!(matches!(cable.exchange(0), Err(Error::SessionLost)));
    }
}

#[test]
fn info_returns_and_refreshes_cached_capacity() {
    let state = Arc::new(Mutex::new(State {
        capacity: 8,
        ..State::default()
    }));
    let mut cable = connect(state.clone());
    assert_eq!(cable.bulk_capacity().unwrap(), 8);
    assert_eq!(cable.bulk_capacity().unwrap(), 8);
    assert_eq!(state.lock().unwrap().requests.len(), 1);

    state.lock().unwrap().capacity = 4;
    assert_eq!(cable.info().unwrap().bulk_capacity, 4);
    assert_eq!(cable.bulk_capacity().unwrap(), 4);
    assert!(matches!(
        cable.bulk_read(5, 0),
        Err(Error::BulkCapacity {
            requested: 5,
            maximum: 4
        })
    ));
    assert!(matches!(
        cable.bulk_write(&[0; 5]),
        Err(Error::BulkCapacity {
            requested: 5,
            maximum: 4
        })
    ));
    {
        let state = state.lock().unwrap();
        assert_eq!(
            state.requests.iter().map(|r| r.0).collect::<Vec<_>>(),
            [8, 8]
        );
    }
    cable.close().unwrap();
    assert!(matches!(cable.bulk_capacity(), Err(Error::SessionLost)));
}

#[test]
fn invalid_info_capacity_rejects_connection_before_any_gba_transfer() {
    for capacity in [0, 257, u32::MAX as usize] {
        let state = Arc::new(Mutex::new(State {
            capacity,
            ..State::default()
        }));
        assert!(matches!(
            Cable::connect(Mock(state.clone()), options()),
            Err(Error::Protocol {
                command: Command::Info,
                ..
            })
        ));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.iter().map(|r| r.0).collect::<Vec<_>>(), [8]);
        assert_eq!(state.dtr.last(), Some(&false));
    }
}

#[test]
fn listing_filters_exact_product_names_and_sorts_both_modes() {
    let port = |name: &str, product: Option<&str>| PortInfo {
        name: name.into(),
        product: product.map(str::to_owned),
        serial_number: Some(name.into()),
        usb_id: Some((0x16c0, 0x27dd)),
    };
    let ports = vec![
        port("COM5", Some("Pico GBA Multibooter Clone")),
        port("COM3", Some("Pico GBA Multibooter")),
        port("COM1", None),
        port("COM4", Some("Pico GBA Multiboot")),
        port("COM2", Some("Pico GBA Multibooter")),
    ];
    assert_eq!(
        crate::serial::filter_ports(ports.clone(), false),
        vec![ports[4].clone(), ports[1].clone()]
    );
    assert_eq!(
        crate::serial::filter_ports(ports.clone(), true),
        vec![
            ports[2].clone(),
            ports[4].clone(),
            ports[1].clone(),
            ports[3].clone(),
            ports[0].clone()
        ]
    );
    assert!(crate::serial::filter_ports(vec![ports[0].clone()], false).is_empty());
    for all in [false, true] {
        assert!(crate::serial::filter_ports(Vec::new(), all).is_empty());
    }
}
