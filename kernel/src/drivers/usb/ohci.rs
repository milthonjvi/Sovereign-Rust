//! OHCI (USB 1.0/1.1) memory-mapped register offsets.
pub const REVISION:usize=0x00;pub const CONTROL:usize=0x04;pub const COMMAND_STATUS:usize=0x08;
pub const INTERRUPT_STATUS:usize=0x0c;pub const INTERRUPT_ENABLE:usize=0x10;
pub const HCCA:usize=0x18;pub const PERIOD_CURRENT_ED:usize=0x1c;
pub const CONTROL_HEAD_ED:usize=0x20;pub const CONTROL_CURRENT_ED:usize=0x24;
pub const BULK_HEAD_ED:usize=0x28;pub const BULK_CURRENT_ED:usize=0x2c;
pub const DONE_HEAD:usize=0x30;pub const FRAME_INTERVAL:usize=0x34;
pub const RH_DESCRIPTOR_A:usize=0x48;pub const RH_DESCRIPTOR_B:usize=0x4c;
pub const RH_STATUS:usize=0x50;pub const RH_PORT_STATUS_BASE:usize=0x54;
pub const CONTROL_HCFS_MASK:u32=3<<6;pub const HCFS_RESET:u32=0<<6;pub const HCFS_RESUME:u32=1<<6;pub const HCFS_OPERATIONAL:u32=2<<6;pub const HCFS_SUSPEND:u32=3<<6;
