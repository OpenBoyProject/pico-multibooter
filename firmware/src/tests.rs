use crate::{link::Link, server};
use core::{future::pending, task::Poll};
use embassy_futures::{block_on, poll_once};
use protocol::{Decoder, DeviceInfo, ErrorCode as E, Frame, MAX_FRAME, Request, Response, Words};
use std::{vec, vec::Vec};

#[derive(Default)]
struct Mock {
    sent: Vec<u32>,
    fail_at: Option<usize>,
    stall_at: Option<usize>,
}

impl Link for Mock {
    async fn exchange(&mut self, word: u32) -> Result<u32, E> {
        self.sent.push(word);
        if self.stall_at == Some(self.sent.len()) {
            pending::<()>().await;
        }
        if self.fail_at == Some(self.sent.len()) {
            return Err(E::Disconnected);
        }
        Ok(word ^ 0xaabb_ccdd)
    }
}

fn transact(link: &mut Mock, request: Request<'_>, expected: Response<'_>) {
    let mut encoded = [0; MAX_FRAME];
    let count = request.encode(&mut encoded).unwrap();
    let mut decoder = Decoder::new();
    let mut reply = [0; MAX_FRAME];
    let mut size = 0;
    for &byte in &encoded[..count] {
        if let Some(frame) = decoder.push(byte) {
            size = block_on(server::handle(frame.unwrap(), link, &mut reply)).unwrap();
        }
    }
    assert!(size > 0);
    let mut found = false;
    for &byte in &reply[..size] {
        if let Some(frame) = decoder.push(byte) {
            assert_eq!(Response::decode_for(request, frame.unwrap()), Ok(expected));
            found = true;
        }
    }
    assert!(found);
}

#[test]
fn info_and_capacity_do_not_clock_spi() {
    let mut link = Mock::default();
    transact(
        &mut link,
        Request::Info,
        Response::Info(DeviceInfo { bulk_capacity: 256 }),
    );
    assert!(link.sent.is_empty());
}

#[test]
fn single_exchange_sends_exact_word() {
    let mut link = Mock::default();
    transact(
        &mut link,
        Request::Exchange(0x12345678),
        Response::Exchange(0x12345678 ^ 0xaabbccdd),
    );
    assert_eq!(link.sent, [0x12345678]);
}

#[test]
fn bulk_operations_preserve_words_and_counts() {
    for count in [1, 256] {
        let mut link = Mock::default();
        let data = 0x6381u32.to_le_bytes().repeat(count); // BIOS-looking words are just data.
        let reply = (0x6381u32 ^ 0xaabbccdd).to_le_bytes().repeat(count);
        let words = Words::from_le_bytes(&data).unwrap();
        let received = Words::from_le_bytes(&reply).unwrap();
        transact(
            &mut link,
            Request::BulkWrite { words },
            Response::Written(data.len() as u32),
        );
        transact(
            &mut link,
            Request::BulkExchange { words },
            Response::BulkData(received),
        );
        transact(
            &mut link,
            Request::BulkRead {
                count: count as u16,
                fill: 0x6381,
            },
            Response::BulkData(received),
        );
        assert_eq!(link.sent, vec![0x6381; count * 3]);
    }
}

#[test]
fn failed_bulk_transfer_returns_no_partial_success_and_does_not_retry() {
    let words = Words::from_le_bytes(&[0; 12]).unwrap();
    for request in [
        Request::BulkRead { count: 3, fill: 0 },
        Request::BulkWrite { words },
        Request::BulkExchange { words },
    ] {
        let mut link = Mock {
            fail_at: Some(2),
            ..Mock::default()
        };
        transact(&mut link, request, Response::Error(E::Disconnected));
        assert_eq!(link.sent, [0, 0]);
    }
}

#[test]
fn removed_commands_and_bad_payloads_never_clock_spi() {
    let mut link = Mock::default();
    let mut output = [0; MAX_FRAME];
    for frame in [
        Frame {
            command: 2,
            payload: &[0; 196],
        },
        Frame {
            command: 4,
            payload: &[],
        },
        Frame {
            command: 5,
            payload: &[],
        },
        Frame {
            command: 9,
            payload: &[],
        },
        Frame {
            command: 3,
            payload: &[2, 192, 0, 0, 0, 1, 2, 3, 4],
        },
        Frame {
            command: 7,
            payload: &[0; 7],
        },
        Frame {
            command: 3,
            payload: &[0; 8],
        },
        Frame {
            command: 99,
            payload: &[],
        },
    ] {
        let size = block_on(server::handle(frame, &mut link, &mut output)).unwrap();
        assert_eq!(size, 12);
        assert_eq!(output[4], frame.command | 0x80);
        assert_eq!(output[7], E::InvalidRequest as u8);
    }
    let size = server::reject(3, E::InvalidFrame, &mut output).unwrap();
    assert_eq!(size, 12);
    assert_eq!(output[7], E::InvalidFrame as u8);
    assert!(link.sent.is_empty());
}

#[test]
fn cancelled_bulk_future_does_not_publish_partial_reply() {
    let mut link = Mock {
        stall_at: Some(2),
        ..Mock::default()
    };
    let mut output = [0xa5; MAX_FRAME];
    let payload = [3, 0, 0, 0, 0, 0];
    assert!(matches!(
        poll_once(server::handle(
            Frame {
                command: 5,
                payload: &payload
            },
            &mut link,
            &mut output
        )),
        Poll::Pending
    ));
    assert_eq!(link.sent, [0, 0]);
    assert_eq!(output, [0xa5; MAX_FRAME]);
    transact(
        &mut link,
        Request::Info,
        Response::Info(DeviceInfo { bulk_capacity: 256 }),
    );
    assert_eq!(link.sent.len(), 2);
}
