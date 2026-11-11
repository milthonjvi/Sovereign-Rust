//! UHCI (USB 1.0/1.1) legacy host-controller register definitions.
pub const USBCMD:u16=0x00;pub const USBSTS:u16=0x02;pub const USBINTR:u16=0x04;
pub const FRNUM:u16=0x06;pub const FLBASEADD:u16=0x08;pub const SOFMOD:u16=0x0c;
pub const PORTSC1:u16=0x10;pub const PORTSC2:u16=0x12;
pub const CMD_RUN:u16=1<<0;pub const CMD_HCRESET:u16=1<<1;pub const CMD_GRESET:u16=1<<2;
pub const STS_USBINT:u16=1<<0;pub const STS_USBERRINT:u16=1<<1;pub const STS_RESUME:u16=1<<2;
pub const STS_HSE:u16=1<<3;pub const STS_HCPE:u16=1<<4;pub const STS_HCHALTED:u16=1<<5;
pub const PORT_CONNECT:u16=1<<0;pub const PORT_CONNECT_CHANGE:u16=1<<1;
pub const PORT_ENABLE:u16=1<<2;pub const PORT_ENABLE_CHANGE:u16=1<<3;
pub const PORT_LOW_SPEED:u16=1<<8;pub const PORT_RESET:u16=1<<9;
