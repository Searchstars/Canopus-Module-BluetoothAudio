//! Exact-target device archive for the Canopus headphone manager.
#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

mod module;

#[cfg(feature = "device")]
pub mod target;

pub use module::*;

#[cfg(not(test))]
#[panic_handler]
fn canopus_panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
