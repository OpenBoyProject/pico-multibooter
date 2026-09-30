use protocol::{
    DecodeError, Decoder, DeviceInfo, Error, ErrorCode, Frame, MAX_BULK_WORDS, MAX_DATA_LEN,
    MAX_FRAME, MAX_PAYLOAD, Request, Response, Words, crc32,
};

// Independent fixtures from Python struct.pack and zlib.crc32.
const INFO: &[u8] = &[
    0x50, 0x4d, 0x42, 0x33, 0x08, 0x00, 0x00, 0xaa, 0x88, 0x52, 0xf1,
];
const INFO_REPLY: &[u8] = &[
    0x50, 0x4d, 0x42, 0x33, 0x88, 0x0d, 0x00, 0x00, 0x50, 0x49, 0x43, 0x4f, 0x2d, 0x4d, 0x42, 0x33,
    0x00, 0x01, 0x00, 0x00, 0xc9, 0x26, 0xff, 0x38,
];
const DATA: &[u8] = &[
    0x50, 0x4d, 0x42, 0x33, 0x06, 0x04, 0x00, 0x00, 0x0a, 0xff, 0x80, 0xd6, 0x53, 0x26, 0x16,
];
const EXCHANGE: &[u8] = &[
    0x50, 0x4d, 0x42, 0x33, 0x07, 0x08, 0x00, 0x78, 0x56, 0x34, 0x12, 0xdd, 0xcc, 0xbb, 0xaa, 0x8f,
    0x00, 0xa9, 0x96,
];
const READ: &[u8] = &[
    0x50, 0x4d, 0x42, 0x33, 0x05, 0x06, 0x00, 0x00, 0x01, 0x78, 0x56, 0x34, 0x12, 0x18, 0x29, 0xb8,
    0xd9,
];

fn inspect(bytes: &[u8], mut check: impl FnMut(Frame<'_>)) {
    let mut decoder = Decoder::new();
    let mut frames = 0;
    for &byte in bytes {
        if let Some(result) = decoder.push(byte) {
            check(result.unwrap());
            frames += 1;
        }
    }
    assert_eq!(frames, 1);
}

fn round_trip(request: Request<'_>, response: Response<'_>) {
    let mut output = [0; MAX_FRAME];
    let length = request.encode(&mut output).unwrap();
    inspect(&output[..length], |frame| {
        assert_eq!(Request::decode(frame), Ok(request));
    });
    let length = response.encode_for(request, &mut output).unwrap();
    inspect(&output[..length], |frame| {
        assert_eq!(Response::decode_for(request, frame), Ok(response));
    });
}

#[test]
fn independent_crc_and_wire_fixtures() {
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    assert_eq!(crc32(b""), 0);
    let mut storage = [0; 8];
    let words = Words::from_words(&[0x12345678, 0xaabbccdd], &mut storage).unwrap();
    let data = Words::from_le_bytes(&[0, 10, 255, 128]).unwrap();
    for (request, fixture) in [
        (Request::Info, INFO),
        (Request::BulkWrite { words: data }, DATA),
        (Request::BulkExchange { words }, EXCHANGE),
        (
            Request::BulkRead {
                count: 256,
                fill: 0x12345678,
            },
            READ,
        ),
    ] {
        let mut encoded = [0; MAX_FRAME];
        let size = request.encode(&mut encoded).unwrap();
        assert_eq!(&encoded[..size], fixture);
        inspect(fixture, |frame| {
            assert_eq!(Request::decode(frame), Ok(request))
        });
    }
    let mut encoded = [0; MAX_FRAME];
    let size = Response::Info(DeviceInfo { bulk_capacity: 256 })
        .encode_for(Request::Info, &mut encoded)
        .unwrap();
    assert_eq!(&encoded[..size], INFO_REPLY);
    inspect(INFO_REPLY, |frame| {
        assert_eq!(
            Response::decode_for(Request::Info, frame),
            Ok(Response::Info(DeviceInfo { bulk_capacity: 256 }))
        )
    });
}

#[test]
fn table_crc_matches_bitwise_reference() {
    fn reference(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    for byte in 0..=u8::MAX {
        assert_eq!(crc32(&[byte]), reference(&[byte]));
    }
    // Cover unaligned slices, frame sizes, and a full dumper CRC transcript.
    let data: Vec<u8> = (0..=MAX_BULK_WORDS * 8).map(|i| i as u8).collect();
    for length in 0..data.len() {
        assert_eq!(crc32(&data[1..1 + length]), reference(&data[1..1 + length]));
    }
    for byte in [0, 0xff] {
        let data = [byte; MAX_FRAME];
        assert_eq!(crc32(&data), reference(&data));
    }
}

#[test]
fn every_packet_split_and_back_to_back_frames() {
    let stream: Vec<_> = INFO.iter().chain(DATA).chain(INFO_REPLY).copied().collect();
    for split in 0..=stream.len() {
        let mut decoder = Decoder::new();
        let mut commands = Vec::new();
        for packet in [&stream[..split], &stream[split..]] {
            for &byte in packet {
                if let Some(result) = decoder.push(byte) {
                    commands.push(result.unwrap().command);
                }
            }
        }
        assert_eq!(commands, [8, 6, 0x88], "split at {split}");
    }
}

#[test]
fn skips_noise_and_partial_magic_without_losing_a_frame() {
    let mut decoder = Decoder::new();
    for &byte in b"garbage\0\xffPPMPMBPPMB" {
        assert!(decoder.push(byte).is_none());
    }
    let mut found = false;
    for &byte in INFO {
        if let Some(result) = decoder.push(byte) {
            assert_eq!(Request::decode(result.unwrap()), Ok(Request::Info));
            found = true;
        }
    }
    assert!(found);
}

#[test]
fn corrupted_frames_and_oversized_headers_recover_at_next_frame() {
    for index in 0..DATA.len() - 7 {
        // Corrupt each payload/CRC byte without changing declared length.
        let mut corrupt = DATA.to_vec();
        corrupt[7 + index] ^= 1;
        let mut decoder = Decoder::new();
        let mut errors = 0;
        let mut valid = 0;
        for byte in corrupt.iter().chain(INFO) {
            if let Some(result) = decoder.push(*byte) {
                match result {
                    Err(DecodeError {
                        command: 6,
                        error: Error::ChecksumMismatch { .. },
                    }) => errors += 1,
                    Ok(frame) => {
                        assert_eq!(frame.command, 8);
                        valid += 1;
                    }
                    other => panic!("unexpected result {other:?}"),
                }
            }
        }
        assert_eq!((errors, valid), (1, 1));
    }

    for length in [MAX_PAYLOAD as u16 + 1, u16::MAX] {
        let mut decoder = Decoder::new();
        let mut header = *b"PMB3\x06\0\0";
        header[5..].copy_from_slice(&length.to_le_bytes());
        for &byte in &header[..6] {
            assert!(decoder.push(byte).is_none());
        }
        assert_eq!(
            decoder.push(header[6]),
            Some(Err(DecodeError {
                command: 6,
                error: Error::PayloadTooLarge
            }))
        );
        for &byte in &INFO[..INFO.len() - 1] {
            assert!(decoder.push(byte).is_none());
        }
        assert!(decoder.push(*INFO.last().unwrap()).unwrap().is_ok());
    }
}

#[test]
fn reset_discards_any_truncated_frame() {
    for length in 0..DATA.len() {
        let mut decoder = Decoder::new();
        for &byte in &DATA[..length] {
            assert!(decoder.push(byte).is_none());
        }
        decoder.reset();
        for &byte in &INFO[..INFO.len() - 1] {
            assert!(decoder.push(byte).is_none());
        }
        assert_eq!(
            Request::decode(decoder.push(*INFO.last().unwrap()).unwrap().unwrap()),
            Ok(Request::Info)
        );
    }
}

#[test]
fn largest_frames_and_embedded_magic() {
    let mut payload = [0xff; MAX_PAYLOAD];
    payload[500..504].copy_from_slice(b"PMB3");
    let frame = Frame {
        command: 0x7f,
        payload: &payload,
    };
    let mut encoded = [0; MAX_FRAME];
    assert_eq!(frame.encode(&mut encoded), Ok(MAX_FRAME));
    inspect(&encoded, |decoded| {
        assert_eq!(decoded, frame);
        assert_eq!(Request::decode(decoded), Err(Error::UnknownCommand(0x7f)));
    });
}

#[test]
fn word_conversion_checks_lengths_and_supports_unaligned_bytes() {
    for size in [0, 1, 3, 5, MAX_BULK_WORDS * 4 + 4] {
        assert_eq!(
            Words::from_le_bytes(&vec![0; size]),
            Err(Error::InvalidWordLength)
        );
    }
    let bytes = [0xff, 0x78, 0x56, 0x34, 0x12];
    assert_eq!(
        Words::from_le_bytes(&bytes[1..]).unwrap().iter().next(),
        Some(0x1234_5678)
    );
    let mut buffer = [0xa5; 3];
    assert_eq!(
        Words::from_words(&[42], &mut buffer),
        Err(Error::BufferTooSmall { needed: 4 })
    );
    assert_eq!(buffer, [0xa5; 3]);
    assert_eq!(
        Words::from_words(&[], &mut buffer),
        Err(Error::InvalidWordLength)
    );
    assert_eq!(
        Words::from_words(&[0; MAX_BULK_WORDS + 1], &mut buffer),
        Err(Error::InvalidWordLength)
    );
}

#[test]
fn round_trips_all_commands_and_boundary_payloads() {
    round_trip(
        Request::Info,
        Response::Info(DeviceInfo { bulk_capacity: 256 }),
    );
    round_trip(Request::Exchange(u32::MAX), Response::Exchange(0));
    round_trip(
        Request::Info,
        Response::Info(DeviceInfo { bulk_capacity: 1 }),
    );

    for count in [1, MAX_BULK_WORDS] {
        let data = vec![0xa5; count * 4];
        let words = Words::from_le_bytes(&data).unwrap();
        round_trip(
            Request::BulkWrite { words },
            Response::Written(data.len() as u32),
        );
        round_trip(Request::BulkExchange { words }, Response::BulkData(words));
        round_trip(
            Request::BulkRead {
                count: count as u16,
                fill: u32::MAX,
            },
            Response::BulkData(words),
        );
    }

    for code in [
        ErrorCode::InvalidRequest,
        ErrorCode::TransferFailed,
        ErrorCode::Disconnected,
        ErrorCode::InvalidFrame,
    ] {
        round_trip(Request::Info, Response::Error(code));
    }
}

#[test]
fn malformed_requests_and_removed_commands_are_rejected() {
    for command in [1, 2, 4, 9, 0x88, 255] {
        assert_eq!(
            Request::decode(Frame {
                command,
                payload: &[]
            }),
            Err(Error::UnknownCommand(command))
        );
    }
    assert!(
        Request::decode(Frame {
            command: 8,
            payload: &[0]
        })
        .is_err()
    );
    for length in [0, 1, 3, 5, 8, 1024] {
        assert!(
            Request::decode(Frame {
                command: 3,
                payload: &vec![0; length]
            })
            .is_err()
        );
    }
    for command in [6, 7] {
        for length in [0, 1, 3, 5, 6, MAX_PAYLOAD, MAX_DATA_LEN + 4] {
            assert!(
                Request::decode(Frame {
                    command,
                    payload: &vec![0; length]
                })
                .is_err()
            );
        }
    }
    // Old bulk requests with a timing byte must be rejected, not shifted.
    for (command, payload) in [
        (6, &[0, 0x78, 0x56, 0x34, 0x12][..]),
        (7, &[1, 0x78, 0x56, 0x34, 0x12][..]),
        (5, &[1, 1, 0, 0, 0, 0, 0][..]),
    ] {
        assert!(Request::decode(Frame { command, payload }).is_err());
    }
    for count in [0u16, 257, u16::MAX] {
        assert_eq!(
            Request::BulkRead { count, fill: 0 }.validate(),
            Err(Error::InvalidWordLength)
        );
        let mut payload = [0; 6];
        payload[..2].copy_from_slice(&count.to_le_bytes());
        assert!(
            Request::decode(Frame {
                command: 5,
                payload: &payload
            })
            .is_err()
        );
    }
}

#[test]
fn replies_require_exact_command_shape_and_word_count() {
    let words = Words::from_le_bytes(&[0; 8]).unwrap();
    for request in [
        Request::BulkRead { count: 2, fill: 0 },
        Request::BulkExchange { words },
        Request::BulkWrite { words },
    ] {
        assert_eq!(
            Response::decode_for(
                request,
                Frame {
                    command: request.command().response_byte(),
                    payload: &[0; 5]
                }
            ),
            Err(Error::InvalidPayload)
        );
    }
    for count in [0u32, MAX_BULK_WORDS as u32 + 1, u32::MAX] {
        let mut output = [0xa5; MAX_FRAME];
        assert_eq!(
            Response::Info(DeviceInfo {
                bulk_capacity: count
            })
            .encode_for(Request::Info, &mut output),
            Err(Error::InvalidPayload)
        );
        assert_eq!(output, [0xa5; MAX_FRAME]);
        let mut payload = [0; 13];
        payload[1..9].copy_from_slice(b"PICO-MB3");
        payload[9..].copy_from_slice(&count.to_le_bytes());
        assert_eq!(
            Response::decode_for(
                Request::Info,
                Frame {
                    command: 0x88,
                    payload: &payload
                }
            ),
            Err(Error::InvalidPayload)
        );
    }
    for payload in [
        &b""[..],
        &b"\0PICO-MB2\0\x01\0\0"[..],
        &b"\0PICO-MB3"[..],
        &b"\0PICO-MB3\0\x01\0\0\0"[..],
    ] {
        assert_eq!(
            Response::decode_for(
                Request::Info,
                Frame {
                    command: 0x88,
                    payload
                }
            ),
            Err(Error::InvalidPayload)
        );
    }
    assert_eq!(
        Response::decode_for(
            Request::Info,
            Frame {
                command: 0x83,
                payload: &[0]
            }
        ),
        Err(Error::UnexpectedCommand {
            expected: 0x88,
            received: 0x83
        })
    );
    for status in [5, 6, 7, 8, 9, 255] {
        assert_eq!(
            Response::decode_for(
                Request::Info,
                Frame {
                    command: 0x88,
                    payload: &[status]
                }
            ),
            Err(Error::UnknownStatus(status))
        );
    }
    assert_eq!(
        Response::decode_for(
            Request::Info,
            Frame {
                command: 0x88,
                payload: &[1, 0]
            }
        ),
        Err(Error::InvalidPayload)
    );
}

#[test]
fn output_is_unchanged_when_encoding_fails() {
    let mut buffer = [0xa5; MAX_FRAME];
    assert_eq!(
        Frame {
            command: 8,
            payload: &[0; MAX_PAYLOAD + 1]
        }
        .encode(&mut buffer),
        Err(Error::PayloadTooLarge)
    );
    for capacity in 0..INFO.len() {
        assert_eq!(
            Request::Info.encode(&mut buffer[..capacity]),
            Err(Error::BufferTooSmall { needed: INFO.len() })
        );
    }
    assert_eq!(
        Request::BulkRead { count: 0, fill: 0 }.encode(&mut buffer),
        Err(Error::InvalidWordLength)
    );
    assert_eq!(
        Response::Written(4).encode_for(Request::Info, &mut buffer),
        Err(Error::InvalidPayload)
    );
    assert_eq!(buffer, [0xa5; MAX_FRAME]);
}

#[test]
fn full_bulk_payload_fits_exact_frame_limit() {
    let data = [0; MAX_DATA_LEN];
    let words = Words::from_le_bytes(&data).unwrap();
    let mut encoded = [0; MAX_FRAME];
    for request in [
        Request::BulkWrite { words },
        Request::BulkExchange { words },
    ] {
        assert_eq!(request.encode(&mut encoded), Ok(MAX_FRAME - 1));
    }
    let request = Request::BulkRead {
        count: MAX_BULK_WORDS as u16,
        fill: 0,
    };
    assert_eq!(
        Response::BulkData(words).encode_for(request, &mut encoded),
        Ok(MAX_FRAME)
    );
}

#[test]
fn arbitrary_command_payload_combinations_do_not_panic() {
    let zeros = [0; MAX_PAYLOAD];
    for command in 0..=u8::MAX {
        for length in [0, 1, 3, 4, 5, 6, 7, 12, 13, 196, 1024, MAX_PAYLOAD] {
            let frame = Frame {
                command,
                payload: &zeros[..length],
            };
            let _ = Request::decode(frame);
            for request in [Request::Info, Request::Exchange(0), Request::Info] {
                let _ = Response::decode_for(request, frame);
            }
        }
    }
}

#[test]
fn old_protocol_frames_do_not_execute() {
    for old_info in [
        b"PMB1\x01\0\0\x25\xb3\x83\xfe",
        b"PMB2\x01\0\0\x25\xb3\x83\xfe",
    ] {
        let mut decoder = Decoder::new();
        for &byte in old_info {
            assert!(decoder.push(byte).is_none());
        }
        let mut found = false;
        for &byte in INFO {
            if let Some(frame) = decoder.push(byte) {
                assert_eq!(Request::decode(frame.unwrap()), Ok(Request::Info));
                found = true;
            }
        }
        assert!(found);
    }
}
