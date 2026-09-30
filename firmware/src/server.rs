//! PMB3 command dispatch. All outgoing SPI words come from the host unchanged.

use crate::link::Link;
use protocol::{
    DeviceInfo, Error, ErrorCode, Frame, MAX_BULK_WORDS, RESPONSE_BIT, Request, Response, Words,
};

/// Process one CRC-checked frame. Cancellation drops any partial reply.
pub async fn handle(
    frame: Frame<'_>,
    link: &mut impl Link,
    output: &mut [u8],
) -> Result<usize, Error> {
    let request = match Request::decode(frame) {
        Ok(request) => request,
        Err(_) => return reject(frame.command, ErrorCode::InvalidRequest, output),
    };
    let mut received = [0; MAX_BULK_WORDS * 4];
    let response = execute(request, link, &mut received)
        .await
        .unwrap_or_else(Response::Error);
    response.encode_for(request, output)
}

/// Reject a malformed frame or unknown command without clocking SPI.
pub fn reject(command: u8, code: ErrorCode, output: &mut [u8]) -> Result<usize, Error> {
    Frame {
        command: command | RESPONSE_BIT,
        payload: &[code as u8],
    }
    .encode(output)
}

async fn execute<'a>(
    request: Request<'_>,
    link: &mut impl Link,
    received: &'a mut [u8; MAX_BULK_WORDS * 4],
) -> Result<Response<'a>, ErrorCode> {
    match request {
        Request::Info => Ok(Response::Info(DeviceInfo {
            bulk_capacity: MAX_BULK_WORDS as u32,
        })),
        Request::Exchange(word) => Ok(Response::Exchange(link.exchange(word).await?)),
        Request::BulkWrite { words } => {
            for word in words.iter() {
                link.exchange(word).await?;
            }
            Ok(Response::Written(words.as_le_bytes().len() as u32))
        }
        Request::BulkRead { count, fill } => {
            let bytes = &mut received[..usize::from(count) * 4];
            for slot in bytes.as_chunks_mut::<4>().0 {
                slot.copy_from_slice(&link.exchange(fill).await?.to_le_bytes());
            }
            Ok(Response::BulkData(Words::from_le_bytes(bytes).unwrap()))
        }
        Request::BulkExchange { words } => {
            let bytes = &mut received[..words.as_le_bytes().len()];
            for (slot, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(words.iter()) {
                slot.copy_from_slice(&link.exchange(word).await?.to_le_bytes());
            }
            Ok(Response::BulkData(Words::from_le_bytes(bytes).unwrap()))
        }
    }
}
