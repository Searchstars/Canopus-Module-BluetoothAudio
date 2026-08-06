use crate::Address;

pub const DISCOVERY_CAPACITY: usize = 12;
pub const DEVICE_NAME_CAPACITY: usize = 64;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DeviceName {
    bytes: [u8; DEVICE_NAME_CAPACITY],
    len: u8,
}

impl Default for DeviceName {
    fn default() -> Self {
        Self {
            bytes: [0; DEVICE_NAME_CAPACITY],
            len: 0,
        }
    }
}

impl DeviceName {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut out = Self::default();
        let mut len = bytes.len().min(DEVICE_NAME_CAPACITY);
        while len > 0 && bytes[len - 1] == 0 {
            len -= 1;
        }
        out.bytes[..len].copy_from_slice(&bytes[..len]);
        out.len = len as u8;
        out
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("Unknown headset")
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub address: Address,
    pub name: DeviceName,
    pub rssi: i32,
    pub class_of_device: u32,
    pub last_seen_epoch: u32,
}

impl DiscoveredDevice {
    /// Bluetooth CoD service bit 21 (Audio) or major class 0x04 (Audio/Video).
    pub const fn audio_likely(&self) -> bool {
        (self.class_of_device & (1 << 21)) != 0 || ((self.class_of_device >> 8) & 0x1f) == 0x04
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryTable {
    entries: [DiscoveredDevice; DISCOVERY_CAPACITY],
    len: u8,
    epoch: u32,
    dropped: u32,
}

impl Default for DiscoveryTable {
    fn default() -> Self {
        Self {
            entries: [DiscoveredDevice::default(); DISCOVERY_CAPACITY],
            len: 0,
            epoch: 0,
            dropped: 0,
        }
    }
}

impl DiscoveryTable {
    pub fn begin_scan(&mut self) -> u32 {
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.len = 0;
        self.dropped = 0;
        self.epoch
    }

    pub const fn epoch(&self) -> u32 {
        self.epoch
    }
    pub const fn len(&self) -> usize {
        self.len as usize
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub const fn dropped(&self) -> u32 {
        self.dropped
    }
    pub fn entries(&self) -> &[DiscoveredDevice] {
        &self.entries[..self.len()]
    }
    pub fn get(&self, index: usize) -> Option<&DiscoveredDevice> {
        self.entries().get(index)
    }

    pub fn upsert(&mut self, mut device: DiscoveredDevice) -> bool {
        if device.address.is_zero() {
            return false;
        }
        device.last_seen_epoch = self.epoch;
        if let Some(index) = self
            .entries()
            .iter()
            .take(self.len())
            .position(|e| e.address == device.address)
        {
            self.entries[index] = device;
            self.sort();
            return true;
        }
        if self.len() == DISCOVERY_CAPACITY {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        let index = self.len();
        self.entries[index] = device;
        self.len += 1;
        self.sort();
        true
    }

    fn sort(&mut self) {
        let len = self.len();
        let mut i = 1;
        while i < len {
            let mut j = i;
            while j > 0 && comes_before(&self.entries[j], &self.entries[j - 1]) {
                self.entries.swap(j, j - 1);
                j -= 1;
            }
            i += 1;
        }
    }
}

fn comes_before(left: &DiscoveredDevice, right: &DiscoveredDevice) -> bool {
    match (left.audio_likely(), right.audio_likely()) {
        (true, false) => true,
        (false, true) => false,
        _ if left.rssi != right.rssi => left.rssi > right.rssi,
        _ => left.address.0 < right.address.0,
    }
}
