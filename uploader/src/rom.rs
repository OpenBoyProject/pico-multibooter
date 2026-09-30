use crate::Error;
/// Multiboot image limits, including the header.
pub const MIN_ROM_SIZE: u32 = 400;
pub const MAX_ROM_SIZE: u32 = 256 * 1024;
pub const ROM_HEADER_LEN: usize = 192;
use std::{fs::File, io::Read, path::Path};

/// Size-checked, zero-padded multiboot image. This does not convert cartridge
/// ROMs or certify that the contents are executable GBA code.
pub struct Rom {
    bytes: Vec<u8>,
    original_len: usize,
}

impl Rom {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::read_from(File::open(path).map_err(|e| Error::io("open ROM", e))?)
    }

    /// Reads at most MAX_ROM_SIZE + 1 bytes, even from an unbounded input.
    pub fn read_from(reader: impl Read) -> Result<Self, Error> {
        let mut bytes = Vec::new();
        reader
            .take(u64::from(MAX_ROM_SIZE) + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::io("read ROM", e))?;
        Self::from_vec(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        check_size(bytes.len())?;
        Self::from_vec(bytes.to_vec())
    }

    fn from_vec(mut bytes: Vec<u8>) -> Result<Self, Error> {
        let original_len = bytes.len();
        check_size(original_len)?;
        bytes.resize(original_len.next_multiple_of(16), 0);
        Ok(Self {
            bytes,
            original_len,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn original_len(&self) -> usize {
        self.original_len
    }
}

fn check_size(length: usize) -> Result<(), Error> {
    if !(MIN_ROM_SIZE as usize..=MAX_ROM_SIZE as usize).contains(&length) {
        return Err(Error::InvalidRomSize(length));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    #[test]
    fn rom_validation_padding_and_bounded_reads() {
        for length in [0, 399, MAX_ROM_SIZE as usize + 1] {
            assert!(matches!(
                Rom::from_bytes(&vec![0; length]),
                Err(Error::InvalidRomSize(_))
            ));
        }
        for length in [
            400,
            401,
            415,
            416,
            MAX_ROM_SIZE as usize - 1,
            MAX_ROM_SIZE as usize,
        ] {
            let input = vec![0xa5; length];
            let rom = Rom::from_bytes(&input).unwrap();
            assert_eq!(rom.original_len(), length);
            assert_eq!(&rom.as_bytes()[..length], input);
            assert!(rom.as_bytes()[length..].iter().all(|&byte| byte == 0));
            assert!(rom.as_bytes().len().is_multiple_of(16));
        }
        assert!(
            matches!(Rom::read_from(io::repeat(0)), Err(Error::InvalidRomSize(size)) if size == MAX_ROM_SIZE as usize + 1)
        );
    }
}
