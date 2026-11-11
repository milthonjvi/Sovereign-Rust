//! UHCI (USB 1.0/1.1) legacy host-controller register definitions.
pub const USBCMD:u16=0x00;pub const USBSTS:u16=0x02;pub const USBINTR:u16=0x04;
pub const FRNUM:u16=0x06;pub const FLBASEADD:u16=0x08;pub const SOFMOD:u16=0x0c;
pub const PORTSC1:u16=0x10;pub const PORTSC2:u16=0x12;
pub const CMD_RUN:u16=1<<0;pub const CMD_HCRESET:u16=1<<рес;
