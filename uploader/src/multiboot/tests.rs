use super::*;
use std::collections::VecDeque;

struct Mock {
    script: VecDeque<(Option<u32>, u32)>,
    sent: Vec<u32>,
    batches: Vec<usize>,
    waits: Vec<Duration>,
    now: Duration,
    word_time: Duration,
    fallback: Option<u32>,
}

impl Default for Mock {
    fn default() -> Self {
        Self {
            script: VecDeque::new(),
            sent: Vec::new(),
            batches: Vec::new(),
            waits: Vec::new(),
            now: Duration::ZERO,
            word_time: Duration::from_micros(161),
            fallback: None,
        }
    }
}

impl Link for Mock {
    fn now(&self) -> Duration {
        self.now
    }
    fn exchange(&mut self, words: &[u32]) -> Result<Vec<u32>, Error> {
        self.batches.push(words.len());
        let mut received = Vec::new();
        for &word in words {
            self.now += self.word_time;
            self.sent.push(word);
            if let Some((expected, reply)) = self.script.pop_front() {
                if let Some(expected) = expected {
                    assert_eq!(word, expected);
                }
                received.push(reply);
            } else {
                received.push(self.fallback.expect("unexpected SPI transfer"));
            }
        }
        Ok(received)
    }
    fn wait(&mut self, duration: Duration) -> Result<(), Error> {
        self.now += duration;
        self.waits.push(duration);
        Ok(())
    }
}

impl Mock {
    fn short(&mut self, word: u16, reply: u16) {
        self.script
            .push_back((Some(u32::from(word)), u32::from(reply) << 16));
    }
    fn start(&mut self, rom: &[u8]) {
        self.short(0x6200, 0x7202);
        self.short(0x6102, 0x7202);
        for (index, bytes) in rom[..192].as_chunks::<2>().0.iter().enumerate() {
            self.short(u16::from_le_bytes(*bytes), ((96 - index) as u16) << 8 | 2);
        }
        self.short(0x6200, 2);
        self.short(0x6202, 0x7202);
        self.short(0x6381, 0x7202);
        self.short(0x6381, 0x7334);
        self.short(0x6443, 0x7334);
        self.short(((rom.len() - 192) / 4 - 0x34) as u16, 0x7356);
    }
    fn body(&mut self, length: usize) {
        for offset in (192..length).step_by(4) {
            self.script
                .push_back((None, (offset as u32 & 0xffff) << 16));
        }
    }
    fn finish(&mut self, checksum: u16) {
        self.short(0x65, 0x75);
        self.short(0x66, 0x75);
        self.short(checksum, checksum);
    }
    fn golden() -> (Self, Rom) {
        let rom = Rom::from_bytes(&(0..400).map(|n| n as u8).collect::<Vec<_>>()).unwrap();
        let mut mock = Self::default();
        mock.start(rom.as_bytes());
        mock.body(400);
        mock.finish(0xcb78);
        (mock, rom)
    }
}

#[test]
fn batching_preserves_reference_encryption_checksum_and_progress() {
    // Known ciphertext and checksum values for the test ROM.
    for capacity in [1, 8, 256] {
        let (mut link, rom) = Mock::golden();
        let mut events = Vec::new();
        upload(&mut link, &rom, capacity, |event| events.push(event)).unwrap();
        assert_eq!(link.sent[104], 0x5da2_6b5b);
        assert_eq!(link.sent[155], 0x6eea_29ee);
        assert!(link.script.is_empty());
        assert_eq!(link.waits, [HANDSHAKE_DELAY]);
        assert!(link.batches.iter().all(|&n| n <= capacity));
        assert_eq!(events.first().unwrap().phase, UploadPhase::WaitingForGba);
        assert_eq!(events.last().unwrap().phase, UploadPhase::Complete);
        assert_eq!(events.last().unwrap().transferred, 400);
        assert!(
            events
                .windows(2)
                .all(|pair| pair[0].transferred <= pair[1].transferred)
        );
        if capacity == 256 {
            assert_eq!(&link.batches[..3], [1, 99, 1]);
        }
    }
}

#[test]
fn full_rom_crosses_acknowledgement_wrap_without_extra_word_round_trips() {
    let rom = Rom::from_bytes(&vec![0x5a; 0x40000]).unwrap();
    let mut link = Mock::default();
    link.start(rom.as_bytes());
    link.body(rom.as_bytes().len());
    // Stop at final checksum: verify payload size/wrap independently of checksum.
    link.short(0x65, 0x75);
    link.short(0x66, 0x75);
    link.script.push_back((None, 0));
    let result = upload(&mut link, &rom, 256, |_| {});
    assert!(matches!(result, Err(Error::ChecksumMismatch { .. })));
    assert!(link.script.is_empty());
    assert_eq!(link.sent.len(), 104 + (0x40000 - 192) / 4 + 3);
    assert_eq!(link.batches.iter().filter(|&&n| n == 256).count(), 255);
}

#[test]
fn bad_acknowledgement_stops_after_current_batch_without_finish_or_retry() {
    let (mut link, rom) = Mock::golden();
    link.script[105].1 = 0; // Second body word should acknowledge offset 196.
    let mut events = Vec::new();
    let result = upload(&mut link, &rom, 8, |event| events.push(event));
    assert!(matches!(
        result,
        Err(Error::Acknowledgement {
            offset: 196,
            reply: 0
        })
    ));
    assert_eq!(link.sent.len(), 104 + 8);
    assert_eq!(events.last().unwrap().transferred, 192);
    assert!(!events.iter().any(|e| e.phase == UploadPhase::Complete));
}

#[test]
fn checksum_mismatch_never_reports_completion() {
    let (mut link, rom) = Mock::golden();
    link.script.back_mut().unwrap().1 = 0;
    let result = upload(&mut link, &rom, 256, |event| {
        assert_ne!(event.phase, UploadPhase::Complete)
    });
    assert!(matches!(
        result,
        Err(Error::ChecksumMismatch {
            expected: 0xcb78,
            received: 0
        })
    ));
}

#[test]
fn readiness_palette_and_checksum_waits_have_deadlines() {
    let rom = Rom::from_bytes(&[0; 400]).unwrap();
    let mut link = Mock {
        fallback: Some(u32::MAX),
        word_time: Duration::from_secs(1),
        ..Mock::default()
    };
    assert!(matches!(
        upload(&mut link, &rom, 256, |_| {}),
        Err(Error::NotReady {
            probes: 10,
            first_reply: u32::MAX,
            last_reply: u32::MAX
        })
    ));

    let (mut link, rom) = Mock::golden();
    link.script.truncate(101); // Through first palette request.
    link.fallback = Some(0x7202_0000);
    link.word_time = Duration::from_secs(1);
    assert!(matches!(
        upload(&mut link, &rom, 256, |_| {}),
        Err(Error::HandshakeTimeout)
    ));

    let (mut link, rom) = Mock::golden();
    link.script[101].1 = 0xdead_0000;
    assert!(matches!(
        upload(&mut link, &rom, 256, |_| {}),
        Err(Error::HandshakeFailed { reply: 0xdead })
    ));

    let (mut link, rom) = Mock::golden();
    link.script.truncate(156); // Through final payload word.
    link.fallback = Some(0);
    link.word_time = Duration::from_secs(1);
    assert!(matches!(
        upload(&mut link, &rom, 256, |_| {}),
        Err(Error::ChecksumTimeout)
    ));
    assert_eq!(link.waits[1..], [Duration::from_millis(1); 5]);
}

mod wire {
    use super::*;
    use mb_host::{Cancellation, Options};
    use protocol::{Decoder, Frame, MAX_FRAME, Request, Response, Words};
    use std::{
        io::{self, Read, Write},
        sync::{Arc, Mutex},
        thread,
    };

    struct State {
        decoder: Decoder,
        replies: VecDeque<u8>,
        commands: Vec<u8>,
        dtr: Vec<bool>,
        gba: Mock,
        capacity: usize,
        lose_body_reply: bool,
    }

    struct Usb(Arc<Mutex<State>>);
    impl Read for Usb {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let mut state = self.0.lock().unwrap();
            if state.replies.is_empty() {
                drop(state);
                thread::sleep(Duration::from_millis(1));
                return Err(io::ErrorKind::TimedOut.into());
            }
            let count = output.len().min(5).min(state.replies.len());
            for byte in &mut output[..count] {
                *byte = state.replies.pop_front().unwrap();
            }
            Ok(count)
        }
    }
    impl Write for Usb {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut state = self.0.lock().unwrap();
            let count = bytes.len().min(7);
            for &byte in &bytes[..count] {
                let frame = state.decoder.push(byte).map(|frame| {
                    let frame = frame.unwrap();
                    (frame.command, frame.payload.to_vec())
                });
                if let Some((command, payload)) = frame {
                    state.commands.push(command);
                    let request = Request::decode(Frame {
                        command,
                        payload: &payload,
                    })
                    .unwrap();
                    let mut word_bytes = Vec::new();
                    let response = match request {
                        Request::Info => Response::Info(protocol::DeviceInfo {
                            bulk_capacity: state.capacity as u32,
                        }),
                        Request::BulkExchange { words } => {
                            let sent: Vec<_> = words.iter().collect();
                            assert!(sent.len() <= state.capacity);
                            let replies = state.gba.exchange(&sent).unwrap();
                            if state.lose_body_reply && sent[0] == 0x5da2_6b5b {
                                continue;
                            }
                            for word in replies {
                                word_bytes.extend_from_slice(&word.to_le_bytes());
                            }
                            Response::BulkData(Words::from_le_bytes(&word_bytes).unwrap())
                        }
                        _ => panic!("unexpected uploader command: {request:?}"),
                    };
                    let mut encoded = [0; MAX_FRAME];
                    let length = response.encode_for(request, &mut encoded).unwrap();
                    state.replies.extend(&encoded[..length]);
                }
            }
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            panic!("unbounded USB flush")
        }
    }
    impl Transport for Usb {
        fn set_timeout(&mut self, _: Duration) -> io::Result<()> {
            Ok(())
        }
        fn set_dtr(&mut self, value: bool) -> io::Result<()> {
            self.0.lock().unwrap().dtr.push(value);
            Ok(())
        }
        fn clear_input(&mut self) -> io::Result<()> {
            self.0.lock().unwrap().replies.clear();
            Ok(())
        }
    }

    fn setup(capacity: usize) -> (Arc<Mutex<State>>, Rom) {
        let (gba, rom) = Mock::golden();
        (
            Arc::new(Mutex::new(State {
                decoder: Decoder::new(),
                replies: VecDeque::new(),
                commands: Vec::new(),
                dtr: Vec::new(),
                gba,
                capacity,
                lose_body_reply: false,
            })),
            rom,
        )
    }
    fn options() -> Options {
        Options {
            reset_delay: Duration::ZERO,
            ..Options::default()
        }
    }

    #[test]
    fn public_uploader_uses_only_raw_bulk_exchanges_over_fragmented_usb() {
        for capacity in [8, 256] {
            let (state, rom) = setup(capacity);
            let mut cable = Cable::connect(Usb(state.clone()), options()).unwrap();
            let mut complete = false;
            upload_rom(&mut cable, &rom, |event| {
                if event.phase == UploadPhase::Complete {
                    assert!(state.lock().unwrap().gba.script.is_empty());
                    complete = true;
                }
            })
            .unwrap();
            assert!(complete);
            assert!(cable.is_usable());
            let state = state.lock().unwrap();
            assert_eq!(state.commands[0], 8);
            assert!(state.commands[1..].iter().all(|&command| command == 7));
            assert_eq!(state.gba.sent[104], 0x5da2_6b5b);
            assert_eq!(state.gba.sent[155], 0x6eea_29ee);
        }
    }

    #[test]
    fn bios_failure_lost_reply_and_cancellation_close_session_without_retry() {
        for failure in 0..3 {
            let (state, rom) = setup(8);
            let mut opts = options();
            opts.command_timeout = Duration::from_millis(20);
            let cancellation: Cancellation = opts.cancellation.clone();
            match failure {
                0 => state.lock().unwrap().gba.script[105].1 = 0,
                1 => state.lock().unwrap().lose_body_reply = true,
                _ => {}
            }
            let mut cable = Cable::connect(Usb(state.clone()), opts).unwrap();
            let error = upload_rom(&mut cable, &rom, |event| {
                assert_ne!(event.phase, UploadPhase::Complete);
                if failure == 2 && event.phase == UploadPhase::Transferring {
                    cancellation.cancel();
                }
            })
            .unwrap_err();
            match failure {
                0 => assert!(matches!(error, Error::Acknowledgement { offset: 196, .. })),
                1 => assert!(matches!(
                    error,
                    Error::Cable(mb_host::Error::Timeout { .. })
                )),
                _ => assert!(matches!(error, Error::Cable(mb_host::Error::Cancelled))),
            }
            assert!(!cable.is_usable());
            let state = state.lock().unwrap();
            assert_eq!(state.gba.sent.len(), if failure == 2 { 104 } else { 112 });
            assert!(!state.dtr.last().unwrap());
        }
    }
}
