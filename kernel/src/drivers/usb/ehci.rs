//! EHCI (USB 2.0 high-speed) capability and operational register offsets.
pub const CAPLENGTH:usize=0x00;pub const HCIVERSION:usize=0x02;pub const HCSPARAMS:usize=0x04;
pub const HCCPARAMS:usize=0x08;pub const HCSP_PORTROUTE:usize=0x0c;
pub const USBCMD:usize=0x00;pub const USBSTS:usize=0x04;pub const USBINTR:usize=0x08;
pub const FRINDEX:usize=0x0c;pub const CTRLDSSEGMENT:usize=0x10;pub const PERIODICLISTBASE:usize=0x14;
pub const ASYNCLISTADDR:usize=0x18;pub const CONFIGFLAG:usize=0x40;pub const PORTSC_BASE:usize=0x44;
pub const CMD_RUN:u32=1;pub const CMD_RESET:u32=1<<1;pub const STS_HALTED:u32=1<<12;
