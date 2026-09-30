//! GBA normal-mode multiboot sender. Batches are validated on the PC.

use crate::ROM_HEADER_LEN;
use crate::{Error, Rom};
use mb_host::{Cable, Transport};
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(1);
const CHECKSUM_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_DELAY: Duration = Duration::from_micros(62_500);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadPhase {
    WaitingForGba,
    Transferring,
    Verifying,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadProgress {
    pub phase: UploadPhase,
    /// Bytes acknowledged, including the header and zero padding.
    pub transferred: usize,
    pub total: usize,
}

/// Upload through raw BulkExchange commands. Encryption, BIOS acknowledgement
/// validation and the final checksum all run here on the PC. Firmware retains
/// the 256 kHz clock and 36 us inter-word gap.
///
/// Failure closes the cable session. Restart the GBA before another upload;
/// batches are never replayed. A bad BIOS acknowledgement is detected after its
/// entire batch has been clocked. Success leaves the cable open for application I/O.
pub fn upload_rom<T: Transport>(
    cable: &mut Cable<T>,
    rom: &Rom,
    progress: impl FnMut(UploadProgress),
) -> Result<(), Error> {
    let result = (|| {
        let capacity = cable.bulk_capacity()?;
        upload(
            &mut CableLink {
                cable,
                epoch: Instant::now(),
            },
            rom,
            capacity,
            progress,
        )
    })();
    if result.is_err() {
        let _ = cable.close();
    }
    result
}

trait Link {
    fn now(&self) -> Duration;
    fn exchange(&mut self, words: &[u32]) -> Result<Vec<u32>, Error>;
    fn wait(&mut self, duration: Duration) -> Result<(), Error>;
}

struct CableLink<'a, T: Transport> {
    cable: &'a mut Cable<T>,
    epoch: Instant,
}

impl<T: Transport> Link for CableLink<'_, T> {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }
    fn exchange(&mut self, words: &[u32]) -> Result<Vec<u32>, Error> {
        Ok(self.cable.bulk_exchange(words)?)
    }
    fn wait(&mut self, duration: Duration) -> Result<(), Error> {
        Ok(self.cable.wait(duration)?)
    }
}

fn exchange(link: &mut impl Link, words: &[u32]) -> Result<Vec<u32>, Error> {
    let reply = link.exchange(words)?;
    if reply.len() != words.len() {
        return Err(Error::InvalidReplyLength {
            expected: words.len(),
            received: reply.len(),
        });
    }
    Ok(reply)
}

fn send16(link: &mut impl Link, word: u16) -> Result<u16, Error> {
    Ok((exchange(link, &[u32::from(word)])?[0] >> 16) as u16)
}

fn upload(
    link: &mut impl Link,
    rom: &Rom,
    capacity: usize,
    mut progress: impl FnMut(UploadProgress),
) -> Result<(), Error> {
    let bytes = rom.as_bytes();
    let total = bytes.len();
    let mut report = |phase, transferred| {
        progress(UploadProgress {
            phase,
            transferred,
            total,
        })
    };
    report(UploadPhase::WaitingForGba, 0);
    let started = link.now();
    let (mut first_reply, mut last_reply, mut probes) = (0, 0, 0);
    loop {
        if link.now() - started >= READY_TIMEOUT {
            return Err(Error::NotReady {
                first_reply,
                last_reply,
                probes,
            });
        }
        let reply = exchange(link, &[0x6200])?[0];
        if probes == 0 {
            first_reply = reply;
        }
        last_reply = reply;
        probes += 1;
        if reply >> 16 == 0x7202 {
            break;
        }
    }

    // Header halfwords occupy one 32-bit SPI exchange each. They can be batched
    // because no response is needed to construct the next outgoing word.
    let mut header = Vec::with_capacity(99);
    header.push(0x6102); // Recognize client 1 (mask 0x02) before sending its header.
    header.extend(
        bytes[..ROM_HEADER_LEN]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|word| u32::from(u16::from_le_bytes(*word))),
    );
    header.extend([0x6200, 0x6202]);
    for chunk in header.chunks(capacity) {
        exchange(link, chunk)?;
    }

    send16(link, 0x6381)?;
    let started = link.now();
    let token = loop {
        if link.now() - started >= HANDSHAKE_TIMEOUT {
            return Err(Error::HandshakeTimeout);
        }
        let reply = send16(link, 0x6381)?;
        if reply >> 8 == 0x73 {
            break reply & 0xff;
        }
        if reply != 0x7202 {
            return Err(Error::HandshakeFailed { reply });
        }
    };
    let mut seed = 0xffff_0000 | (u32::from(token) << 8) | 0x81;
    let handshake = (token + 0x0f) & 0xff;
    send16(link, 0x6400 | handshake)?;
    link.wait(HANDSHAKE_DELAY)?;
    let length = ((total - ROM_HEADER_LEN) / 4 - 0x34) as u16;
    let random = send16(link, length)? & 0xff;
    let final_crc_word = 0xffff_0000 | (u32::from(random) << 8) | u32::from(handshake);
    let mut crc = 0xc387;
    let mut offset = ROM_HEADER_LEN;
    report(UploadPhase::Transferring, offset);
    for chunk in bytes[ROM_HEADER_LEN..].chunks(capacity * 4) {
        let mut encrypted = Vec::with_capacity(chunk.len() / 4);
        for (index, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            let word = u32::from_le_bytes(*word);
            crc = crc_step(crc, word);
            seed = seed.wrapping_mul(0x6f64_6573).wrapping_add(1);
            let address = (offset + index * 4) as u32;
            encrypted.push(word ^ seed ^ 0xfe00_0000u32.wrapping_sub(address) ^ 0x4320_2f2f);
        }
        let replies = exchange(link, &encrypted)?;
        for (index, reply) in replies.into_iter().enumerate() {
            let address = (offset + index * 4) as u32;
            if reply >> 16 != address & 0xffff {
                return Err(Error::Acknowledgement {
                    offset: address,
                    reply,
                });
            }
        }
        offset += chunk.len();
        report(UploadPhase::Transferring, offset);
    }

    report(UploadPhase::Verifying, offset);
    let checksum = crc_step(crc, final_crc_word) as u16;
    let started = link.now();
    loop {
        if link.now() - started >= CHECKSUM_TIMEOUT {
            return Err(Error::ChecksumTimeout);
        }
        if send16(link, 0x65)? == 0x75 {
            break;
        }
        link.wait(Duration::from_millis(1))?;
    }
    send16(link, 0x66)?;
    let received = send16(link, checksum)?;
    if received != checksum {
        return Err(Error::ChecksumMismatch {
            expected: checksum,
            received,
        });
    }
    report(UploadPhase::Complete, offset);
    Ok(())
}

fn crc_step(mut crc: u32, mut word: u32) -> u32 {
    for _ in 0..32 {
        let bit = (crc ^ word) & 1;
        crc >>= 1;
        if bit != 0 {
            crc ^= 0xc37b;
        }
        word >>= 1;
    }
    crc
}

#[cfg(test)]
mod tests;
