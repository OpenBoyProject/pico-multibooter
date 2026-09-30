//! Raw 32-bit SPI link supplied by the hardware layer.

use core::future::Future;
use protocol::ErrorCode;

pub const WORD_DELAY_US: u64 = 36;

/// Exchange must check cancellation and wait at least WORD_DELAY_US afterwards.
pub trait Link {
    fn exchange(&mut self, word: u32) -> impl Future<Output = Result<u32, ErrorCode>>;
}
