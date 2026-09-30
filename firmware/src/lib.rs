#![no_std]

//! Hardware-independent raw cable command handling.
//! The executable supplies the Embassy USB transport, SPI link, and clock.

pub mod link;
pub mod server;

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests;
