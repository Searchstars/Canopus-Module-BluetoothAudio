use core::fmt::{self, Write};

#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Address(pub [u8; 6]);

impl Address {
    pub const ZERO: Self = Self([0; 6]);

    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    pub const fn is_zero(self) -> bool {
        self.0[0] == 0
            && self.0[1] == 0
            && self.0[2] == 0
            && self.0[3] == 0
            && self.0[4] == 0
            && self.0[5] == 0
    }

    pub fn text(self) -> AddressText {
        let mut out = AddressText::default();
        let _ = write!(
            out,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        );
        out
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressText {
    bytes: [u8; 17],
    len: u8,
}

impl AddressText {
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

impl Write for AddressText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let remaining = self.bytes.len().saturating_sub(self.len as usize);
        if value.len() > remaining {
            return Err(fmt::Error);
        }
        let start = self.len as usize;
        self.bytes[start..start + value.len()].copy_from_slice(value.as_bytes());
        self.len += value.len() as u8;
        Ok(())
    }
}
