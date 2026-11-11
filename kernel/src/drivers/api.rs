//! Minimal, allocation-free contracts shared by device drivers.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceClass {
    Display,
    Input,
    Network,
    Storage,
    Serial,
    Timer,
    Usb,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusAddress {
    Pci { segment: u16, bus: u8, device: u8, function: u8 },
    Platform { identifier: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceInfo {
    pub class: DeviceClass,
    pub vendor_id: Option<u16>,
    pub device_id: Option<u16>,
    pub address: BusAddress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriverError {
    UnsupportedDevice,
    ResourceUnavailable,
    InvalidConfiguration,
    HardwareFault,
    NotReady,
}

/// Common lifecycle contract. Resource mapping and interrupt registration are
/// provided by kernel services, not implemented independently by each driver.
pub trait Driver {
    fn name(&self) -> &'static str;
    fn supports(&self, device: &DeviceInfo) -> bool;
    fn initialize(&mut self, device: &DeviceInfo) -> Result<(), DriverError>;
    fn shutdown(&mut self) -> Result<(), DriverError>;
}
