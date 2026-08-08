//! Portable, allocator-free Bluetooth headphone manager.
#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod address;
pub mod avdtp;
pub mod controller;
pub mod discovery;
pub mod media;
pub mod model;
mod sbc_tone_frames;
pub mod ui;

pub use address::{Address, AddressText};
pub use controller::{Controller, Platform};
pub use discovery::{DISCOVERY_CAPACITY, DeviceName, DiscoveredDevice, DiscoveryTable};
pub use model::*;
