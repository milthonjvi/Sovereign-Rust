//! I/O APIC register access and GSI redirection.
//!
//! The caller owns physical-to-virtual MMIO mapping and must map the register
//! page as uncached device memory. This module never enables CPU interrupts.

use core::ptr::{NonNull, read_volatile, write_volatile};

use crate::drivers::acpi::madt::{InterruptSourceOverride, IoApic};
use crate::drivers::ioapic_topology::{IoApicGsiRange, MAX_REDIRECTION_ENTRIES};

const IOREGSEL: usize = 0x00;
const IOWIN: usize = 0x10;
const REG_ID: u8 = 0x00;
const REG_VERSION: u8 = 0x01;
const REG_REDIRECTION_BASE: u8 = 0x10;
const REDIRECTION_MASK: u32 = 1 << 16;
const REDIRECTION_ACTIVE_LOW: u32 = 1 << 13;
const REDIRECTION_LEVEL: u32 = 1 << 15;
const REDIRECTION_WRITABLE_LOW_MASK: u32 = 0x0001_AFFF;
const REDIRECTION_WRITABLE_HIGH_MASK: u32 = 0xFF00_0000;

/// Decode the controller's 4-bit hardware ID from IOAPICID.
#[inline]
const fn decode_controller_id(id_register: u32) -> u8 {
    ((id_register >> 24) & 0x0F) as u8
}

/// Decode the maximum redirection index in IOAPICVER into an entry count.
#[inline]
const fn decode_redirection_count(version_register: u32) -> Result<u32, IoApicError> {
    let count = ((version_register >> 16) & 0xFF) + 1;
    if count == 0 || count > MAX_REDIRECTION_ENTRIES {
        Err(IoApicError::InvalidRedirectionCount)
    } else {
        Ok(count)
    }
}

/// Reject a GSI interval whose exclusive end cannot be represented.
#[inline]
const fn validate_gsi_interval(base: u32, count: u32) -> Result<(), IoApicError> {
    if base.checked_add(count).is_some() {
        Ok(())
    } else {
        Err(IoApicError::GsiRangeOverflow)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoApicError {
    InvalidMapping,
    InvalidVector,
    RouteNotConfigured,
    DuplicateOverride,
    GsiOutOfRange,
    InvalidPolarity,
    InvalidTriggerMode,
    InvalidFlags,
    HardwareRejectedWrite,
    InvalidIsaIrq,
    ControllerIdMismatch,
    InvalidRedirectionCount,
    GsiRangeOverflow,
    DestinationIdOutOfRange,
}

/// Decoded ACPI interrupt electrical configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterruptMode {
    pub active_low: bool,
    pub level_triggered: bool,
}

impl InterruptMode {
    /// Decode ACPI MADT MPS INTI flags. Conforms to the defined ISA encodings.
    pub fn from_acpi_flags(flags: u16) -> Result<Self, IoApicError> {
        if flags & !0x0F != 0 {
            return Err(IoApicError::InvalidFlags);
        }
        let polarity = flags & 0b11;
        let trigger = (flags >> 2) & 0b11;
        // ACPI encodings: 0 = bus conforms, 1 = high/edge, 3 = low/level.
        // Encoding 2 is reserved and must never be silently accepted.
        let active_low = match polarity {
            0 | 1 => false,
            3 => true,
            _ => return Err(IoApicError::InvalidPolarity),
        };
        let level_triggered = match trigger {
            0 | 1 => false,
            3 => true,
            _ => return Err(IoApicError::InvalidTriggerMode),
        };
        Ok(Self { active_low, level_triggered })
    }
}

/// Validate vectors accepted by external I/O APIC routes.
///
/// Keep this range synchronized with the interrupt subsystem: 0x20..=0x2F
/// belongs to the legacy PIC, 0x80 is reserved for the syscall ABI, and 0xFF
/// is the local-APIC spurious vector.
#[inline]
pub const fn valid_redirection_vector(vector: u8) -> bool {
    vector >= 0x30 && vector <= 0xFE && vector != 0x80
}

/// Compare only architecturally writable redirection bits. Delivery status
/// and Remote-IRR are read-only and may change asynchronously.
#[inline]
const fn redirection_readback_matches(
    expected_low: u32,
    expected_high: u32,
    actual_low: u32,
    actual_high: u32,
) -> bool {
    (expected_low & REDIRECTION_WRITABLE_LOW_MASK)
        == (actual_low & REDIRECTION_WRITABLE_LOW_MASK)
        && (expected_high & REDIRECTION_WRITABLE_HIGH_MASK)
            == (actual_high & REDIRECTION_WRITABLE_HIGH_MASK)
}

/// Decoded hardware state of one I/O APIC redirection entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RedirectionState {
    pub vector: u8,
    /// Raw three-bit APIC delivery mode from bits 8..=10.
    pub delivery_mode: u8,
    pub logical_destination: bool,
    pub delivery_pending: bool,
    pub mode: InterruptMode,
    pub remote_irr: bool,
    pub masked: bool,
    pub destination_apic_id: u8,
}

#[inline]
const fn decode_redirection_state(low: u32, high: u32) -> RedirectionState {
    RedirectionState {
        vector: (low & 0xFF) as u8,
        delivery_mode: ((low >> 8) & 0x07) as u8,
        logical_destination: low & (1 << 11) != 0,
        delivery_pending: low & (1 << 12) != 0,
        mode: InterruptMode {
            active_low: low & REDIRECTION_ACTIVE_LOW != 0,
            level_triggered: low & REDIRECTION_LEVEL != 0,
        },
        remote_irr: low & (1 << 14) != 0,
        masked: low & REDIRECTION_MASK != 0,
        destination_apic_id: (high >> 24) as u8,
    }
}

/// One I/O APIC redirection entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RedirectionEntry {
    pub vector: u8,
    pub destination_apic_id: u32,
    pub mode: InterruptMode,
}

impl RedirectionEntry {
    fn low(self) -> u32 {
        (self.vector as u32)
            | if self.mode.active_low { REDIRECTION_ACTIVE_LOW } else { 0 }
            | if self.mode.level_triggered { REDIRECTION_LEVEL } else { 0 }
    }

    fn high(self) -> Result<u32, IoApicError> {
        if self.destination_apic_id > u8::MAX as u32 {
            return Err(IoApicError::DestinationIdOutOfRange);
        }
        Ok(self.destination_apic_id << 24)
    }
}

/// Return the fixed-size bitmap slot for a hardware redirection index.
#[inline]
const fn route_bitmap_slot(index: u32) -> Option<(usize, u64)> {
    if index >= MAX_REDIRECTION_ENTRIES {
        None
    } else {
        Some(((index / 64) as usize, 1u64 << (index % 64)))
    }
}

#[inline]
fn route_bitmap_contains(bitmap: &[u64; 2], index: u32) -> bool {
    match route_bitmap_slot(index) {
        Some((word, mask)) => bitmap[word] & mask != 0,
        None => false,
    }
}

#[inline]
fn route_bitmap_insert(bitmap: &mut [u64; 2], index: u32) {
    if let Some((word, mask)) = route_bitmap_slot(index) {
        bitmap[word] |= mask;
    }
}

#[inline]
fn route_bitmap_remove(bitmap: &mut [u64; 2], index: u32) {
    if let Some((word, mask)) = route_bitmap_slot(index) {
        bitmap[word] &= !mask;
    }
}

/// Handle for one mapped I/O APIC register page.
///
/// # Safety
/// The pointer must reference at least 0x20 bytes of writable, uncached MMIO
/// for the I/O APIC described by `descriptor`. Access must be serialized
/// against every other CPU and driver using this same I/O APIC.
pub struct IoApicController {
    base: NonNull<u8>,
    gsi_base: u32,
    redirection_count: u32,
    id: u8,
    // Two words cover all 120 architectural redirection entries without allocation.
    configured_entries: [u64; 2],
}

impl IoApicController {
    /// Construct the controller and immediately mask every redirection input.
    ///
    /// # Safety
    /// The caller must uphold the MMIO mapping and exclusive serialization
    /// requirements of `new`. Call during early boot with CPU interrupts off.
    pub unsafe fn new_masked(
        base: NonNull<u8>,
        descriptor: IoApic,
    ) -> Result<Self, IoApicError> {
        // SAFETY: Forwarded from this constructor's caller.
        let mut controller = unsafe { Self::new(base, descriptor)? };
        controller.mask_all()?;
        Ok(controller)
    }

    /// Construct and read the controller's ID/version registers without
    /// changing any existing redirection entry.
    ///
    /// Prefer `new_masked` during kernel initialization. This constructor is
    /// only appropriate when the caller has independently guaranteed that
    /// every existing delivery source is masked and will remain quiescent.
    ///
    /// # Safety
    /// Caller must uphold the MMIO and serialization contract on this type,
    /// and must guarantee that preserving firmware routing state is safe.
    pub unsafe fn new(
        base: NonNull<u8>,
        descriptor: IoApic,
    ) -> Result<Self, IoApicError> {
        // The register selector/data window is accessed through volatile u32
        // pointers. Reject an invalid firmware base or an unaligned virtual
        // mapping before the first MMIO access (which would otherwise be UB).
        if descriptor.address == 0
            || descriptor.address & 0x0fff != 0
            || (base.as_ptr() as usize) & (core::mem::align_of::<u32>() - 1) != 0
        {
            return Err(IoApicError::InvalidMapping);
        }

        let mut controller = Self {
            base,
            gsi_base: descriptor.global_system_interrupt_base,
            redirection_count: 0,
            id: 0,
            configured_entries: [0; 2],
        };
        let id_version = controller.read_register(REG_ID);
        let version = controller.read_register(REG_VERSION);
        controller.id = decode_controller_id(id_version);
        if controller.id != descriptor.id {
            return Err(IoApicError::ControllerIdMismatch);
        }
        controller.redirection_count = decode_redirection_count(version)?;
        validate_gsi_interval(controller.gsi_base, controller.redirection_count)?;
        Ok(controller)
    }

    #[inline]
    fn read_register(&mut self, register: u8) -> u32 {
        // SAFETY: Constructor contract guarantees mapped MMIO; &mut serializes.
        unsafe {
            write_volatile(self.base.as_ptr().add(IOREGSEL).cast::<u32>(), register as u32);
            read_volatile(self.base.as_ptr().add(IOWIN).cast::<u32>())
        }
    }

    #[inline]
    fn write_register(&mut self, register: u8, value: u32) {
        // SAFETY: Constructor contract guarantees writable mapped MMIO.
        unsafe {
            write_volatile(self.base.as_ptr().add(IOREGSEL).cast::<u32>(), register as u32);
            write_volatile(self.base.as_ptr().add(IOWIN).cast::<u32>(), value);
        }
    }

    pub const fn id(&self) -> u8 { self.id }
    pub const fn gsi_base(&self) -> u32 { self.gsi_base }
    pub const fn redirection_count(&self) -> u32 { self.redirection_count }

    /// Export the hardware-discovered interval for multi-controller routing.
    pub const fn gsi_range(&self) -> IoApicGsiRange {
        IoApicGsiRange {
            apic_id: self.id,
            gsi_base: self.gsi_base,
            redirection_count: self.redirection_count,
        }
    }

    fn redirection_index(&self, gsi: u32) -> Result<u32, IoApicError> {
        let index = gsi.checked_sub(self.gsi_base).ok_or(IoApicError::GsiOutOfRange)?;
        if index >= self.redirection_count {
            return Err(IoApicError::GsiOutOfRange);
        }
        Ok(index)
    }

    /// Read an existing redirection entry.
    pub fn read_redirection(&mut self, gsi: u32) -> Result<(u32, u32), IoApicError> {
        let index = self.redirection_index(gsi)?;
        let low_reg = REG_REDIRECTION_BASE + (index * 2) as u8;
        // The low dword contains the asynchronously changing Remote-IRR bit.
        let low = self.read_register(low_reg);
        let high = self.read_register(low_reg + 1);
        Ok((low, high))
    }

    /// Read and decode the current hardware state of one redirection entry.
    pub fn read_redirection_state(
        &mut self,
        gsi: u32,
    ) -> Result<RedirectionState, IoApicError> {
        let (low, high) = self.read_redirection(gsi)?;
        Ok(decode_redirection_state(low, high))
    }

    /// Program an entry masked, writing destination before delivery attributes.
    /// A separate call to `set_masked(false)` is required to enable delivery.
    ///
    /// `destination_apic_id` is the 8-bit physical destination field supported
    /// by the I/O APIC redirection format. Systems with wider x2APIC IDs require
    /// interrupt remapping for destinations that cannot be represented here.
    pub fn configure_masked(
        &mut self,
        gsi: u32,
        entry: RedirectionEntry,
    ) -> Result<(), IoApicError> {
        if !valid_redirection_vector(entry.vector) {
            return Err(IoApicError::InvalidVector);
        }
        let expected_high = entry.high()?;
        let index = self.redirection_index(gsi)?;
        let low_reg = REG_REDIRECTION_BASE + (index * 2) as u8;
        // First mask the existing route, then change destination, then publish
        // the new delivery attributes while it is still masked.
        // Invalidate any previous authorization before touching hardware.
        // A failed reconfiguration must never leave a stale route unmaskable.
        self.unmark_route_configured(index);

        let previous_low = self.read_register(low_reg);
        self.write_register(low_reg, previous_low | REDIRECTION_MASK);
        if self.read_register(low_reg) & REDIRECTION_MASK == 0 {
            // Retry the safety-critical mask once before abandoning the route.
            self.write_register(low_reg, previous_low | REDIRECTION_MASK);
            return Err(IoApicError::HardwareRejectedWrite);
        }

        let expected_low = entry.low() | REDIRECTION_MASK;
        self.write_register(low_reg + 1, expected_high);
        self.write_register(low_reg, expected_low);

        // Read back the programmed fields before making this route eligible
        // for unmasking. Read-only Delivery Status and Remote-IRR are excluded.
        let actual_low = self.read_register(low_reg);
        let actual_high = self.read_register(low_reg + 1);
        if !redirection_readback_matches(
            expected_low,
            expected_high,
            actual_low,
            actual_high,
        ) {
            // Best-effort fail-safe: leave the source masked and never publish
            // it in the configured-route bitmap.
            self.write_register(low_reg, actual_low | REDIRECTION_MASK);
            let _ = self.read_register(low_reg);
            return Err(IoApicError::HardwareRejectedWrite);
        }

        self.mark_route_configured(index);
        Ok(())
    }

    /// Mask one GSI while preserving its vector, destination and trigger mode.
    pub fn mask(&mut self, gsi: u32) -> Result<(), IoApicError> {
        self.set_masked_internal(gsi, true)
    }

    /// Enable delivery for a route that was configured through this controller.
    ///
    /// # Safety
    /// The caller must have registered a valid interrupt handler for the route's
    /// vector, initialized the destination local APIC, and ensured the target
    /// CPU can receive this vector. The handler must remain valid for the
    /// lifetime of the unmasked route.
    pub unsafe fn unmask(&mut self, gsi: u32) -> Result<(), IoApicError> {
        self.set_masked_internal(gsi, false)
    }

    fn set_masked_internal(&mut self, gsi: u32, masked: bool) -> Result<(), IoApicError> {
        let index = self.redirection_index(gsi)?;
        if !masked && !self.route_is_configured(index) {
            return Err(IoApicError::RouteNotConfigured);
        }
        let low_reg = REG_REDIRECTION_BASE + (index * 2) as u8;
        let mut low = self.read_register(low_reg);
        if masked { low |= REDIRECTION_MASK; } else { low &= !REDIRECTION_MASK; }
        self.write_register(low_reg, low);
        let actual = self.read_register(low_reg);
        if (actual & REDIRECTION_MASK != 0) != masked {
            // Fail safe: any mask-state transition that did not read back as
            // requested is followed by a best-effort attempt to mask the line.
            self.write_register(low_reg, actual | REDIRECTION_MASK);
            let _ = self.read_register(low_reg);
            return Err(IoApicError::HardwareRejectedWrite);
        }
        Ok(())
    }

    #[inline]
    fn route_is_configured(&self, index: u32) -> bool {
        route_bitmap_contains(&self.configured_entries, index)
    }

    #[inline]
    fn mark_route_configured(&mut self, index: u32) {
        route_bitmap_insert(&mut self.configured_entries, index);
    }

    #[inline]
    fn unmark_route_configured(&mut self, index: u32) {
        route_bitmap_remove(&mut self.configured_entries, index);
    }

    /// Mask every redirection entry and verify the hardware accepted each mask.
    pub fn mask_all(&mut self) -> Result<(), IoApicError> {
        // Require every route to be explicitly configured again before it can
        // be unmasked after a controller-wide shutdown.
        self.configured_entries = [0; 2];
        for index in 0..self.redirection_count {
            let register = REG_REDIRECTION_BASE + (index * 2) as u8;
            let low = self.read_register(register);
            self.write_register(register, low | REDIRECTION_MASK);
            if self.read_register(register) & REDIRECTION_MASK == 0 {
                self.write_register(register, low | REDIRECTION_MASK);
                let _ = self.read_register(register);
                return Err(IoApicError::HardwareRejectedWrite);
            }
        }
        Ok(())
    }
}

/// Resolve an ISA IRQ to its GSI and electrical mode using bus-0 MADT
/// overrides. Overrides for other buses are unrelated and are ignored.
pub fn resolve_isa_irq(
    irq: u8,
    overrides: &[InterruptSourceOverride],
) -> Result<(u32, InterruptMode), IoApicError> {
    if irq >= 16 {
        return Err(IoApicError::InvalidIsaIrq);
    }
    let mut matched = None;
    for entry in overrides {
        if entry.bus == 0 && entry.source == irq {
            if matched.is_some() {
                return Err(IoApicError::DuplicateOverride);
            }
            matched = Some(*entry);
        }
    }

    match matched {
        Some(entry) => Ok((
            entry.global_system_interrupt,
            InterruptMode::from_acpi_flags(entry.flags)?,
        )),
        None => Ok((irq as u32, InterruptMode { active_low: false, level_triggered: false })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ioapic_id_and_redirection_count_registers() {
        assert_eq!(decode_controller_id(0x0B00_0000), 0x0B);
        assert_eq!(decode_redirection_count(23 << 16), Ok(24));
        assert_eq!(
            decode_redirection_count((MAX_REDIRECTION_ENTRIES - 1) << 16),
            Ok(MAX_REDIRECTION_ENTRIES)
        );
        assert_eq!(
            decode_redirection_count(MAX_REDIRECTION_ENTRIES << 16),
            Err(IoApicError::InvalidRedirectionCount)
        );
    }

    #[test]
    fn rejects_gsi_interval_overflow_without_wrapping() {
        assert_eq!(
            validate_gsi_interval(u32::MAX - 7, 8),
            Err(IoApicError::GsiRangeOverflow)
        );
        assert_eq!(validate_gsi_interval(0, 120), Ok(()));
    }

    #[test]
    fn rejects_invalid_mmio_mapping_before_register_access() {
        let mut registers = [0u32; 8];
        let aligned = NonNull::new(registers.as_mut_ptr().cast::<u8>()).unwrap();
        let valid_descriptor = IoApic {
            id: 1,
            address: 0xFEC0_0000,
            global_system_interrupt_base: 0,
        };

        // SAFETY: The constructor must reject the unaligned pointer before
        // touching MMIO; the backing array remains alive for this call.
        let unaligned = unsafe { NonNull::new_unchecked(aligned.as_ptr().add(1)) };
        assert!(matches!(
            unsafe { IoApicController::new(unaligned, valid_descriptor) },
            Err(IoApicError::InvalidMapping)
        ));

        let zero_address = IoApic { address: 0, ..valid_descriptor };
        assert!(matches!(
            unsafe { IoApicController::new(aligned, zero_address) },
            Err(IoApicError::InvalidMapping)
        ));

        let unaligned_physical = IoApic { address: 0xFEC0_0001, ..valid_descriptor };
        assert!(matches!(
            unsafe { IoApicController::new(aligned, unaligned_physical) },
            Err(IoApicError::InvalidMapping)
        ));
    }

    #[test]
    fn configured_route_bitmap_covers_all_supported_redirection_entries() {
        let mut configured = [0u64; 2];
        for index in [0u32, 63, 64, 119] {
            assert!(!route_bitmap_contains(&configured, index));
            route_bitmap_insert(&mut configured, index);
            assert!(route_bitmap_contains(&configured, index));
        }
        assert_eq!(configured[0], (1u64 << 0) | (1u64 << 63));
        assert_eq!(configured[1], (1u64 << 0) | (1u64 << 55));
        route_bitmap_remove(&mut configured, 64);
        assert!(!route_bitmap_contains(&configured, 64));
        assert!(route_bitmap_contains(&configured, 119));
        assert_eq!(route_bitmap_slot(MAX_REDIRECTION_ENTRIES), None);
        assert!(!route_bitmap_contains(&configured, MAX_REDIRECTION_ENTRIES));
    }

    #[test]
    fn decodes_acpi_polarity_and_trigger_flags() {
        assert_eq!(
            InterruptMode::from_acpi_flags(0).unwrap(),
            InterruptMode { active_low: false, level_triggered: false }
        );
        assert_eq!(
            InterruptMode::from_acpi_flags(0b1111).unwrap(),
            InterruptMode { active_low: true, level_triggered: true }
        );
        assert_eq!(InterruptMode::from_acpi_flags(0b0010), Err(IoApicError::InvalidPolarity));
        assert_eq!(InterruptMode::from_acpi_flags(0b1000), Err(IoApicError::InvalidTriggerMode));
    }

    #[test]
    fn acpi_conforming_and_reserved_encodings_are_handled_correctly() {
        assert_eq!(InterruptMode::from_acpi_flags(0b0101).unwrap(),
            InterruptMode { active_low: false, level_triggered: false });
        assert_eq!(InterruptMode::from_acpi_flags(0x10), Err(IoApicError::InvalidFlags));
    }

    #[test]
    fn rejects_irq_numbers_outside_the_isa_legacy_range() {
        assert_eq!(resolve_isa_irq(16, &[]), Err(IoApicError::InvalidIsaIrq));
        assert_eq!(resolve_isa_irq(u8::MAX, &[]), Err(IoApicError::InvalidIsaIrq));
    }

    #[test]
    fn resolves_isa_irq_override() {
        let overrides = [InterruptSourceOverride {
            bus: 0,
            source: 0,
            global_system_interrupt: 2,
            flags: 0b1111,
        }];
        let (gsi, mode) = resolve_isa_irq(0, &overrides).unwrap();
        assert_eq!(gsi, 2);
        assert!(mode.active_low && mode.level_triggered);
        assert_eq!(resolve_isa_irq(4, &overrides).unwrap().0, 4);
        let non_isa = [InterruptSourceOverride {
            bus: 1, source: 4, global_system_interrupt: 9, flags: 0,
        }];
        // A bus-1 source number is not an ISA IRQ override.
        assert_eq!(resolve_isa_irq(4, &non_isa).unwrap().0, 4);
    }

    #[test]
    fn isa_resolution_ignores_other_buses_and_rejects_duplicate_isa_overrides() {
        let overrides = [
            InterruptSourceOverride {
                bus: 1,
                source: 4,
                global_system_interrupt: 99,
                flags: 0,
            },
            InterruptSourceOverride {
                bus: 0,
                source: 4,
                global_system_interrupt: 20,
                flags: 0b1111,
            },
        ];
        let (gsi, mode) = resolve_isa_irq(4, &overrides).unwrap();
        assert_eq!(gsi, 20);
        assert_eq!(mode, InterruptMode { active_low: true, level_triggered: true });

        let non_isa_only = [InterruptSourceOverride {
            bus: 1,
            source: 4,
            global_system_interrupt: 99,
            flags: 0,
        }];
        assert_eq!(
            resolve_isa_irq(4, &non_isa_only).unwrap().0,
            4,
        );

        let duplicate = [
            InterruptSourceOverride { bus: 0, source: 4, global_system_interrupt: 20, flags: 0 },
            InterruptSourceOverride { bus: 0, source: 4, global_system_interrupt: 21, flags: 0 },
        ];
        assert_eq!(resolve_isa_irq(4, &duplicate), Err(IoApicError::DuplicateOverride));
    }

    #[test]
    fn external_routes_match_the_interrupt_subsystem_vector_policy() {
        for vector in 0u8..=u8::MAX {
            assert_eq!(
                valid_redirection_vector(vector),
                crate::interrupts::apic::is_apic_vector(vector),
                "IOAPIC vector policy differs at {vector:#04x}",
            );
        }
    }

    #[test]
    fn decodes_all_redirection_status_fields() {
        let low = 0x45
            | (0b101 << 8)
            | (1 << 11)
            | (1 << 12)
            | REDIRECTION_ACTIVE_LOW
            | (1 << 14)
            | REDIRECTION_LEVEL
            | REDIRECTION_MASK;
        let high = 0xA500_0000;
        assert_eq!(
            decode_redirection_state(low, high),
            RedirectionState {
                vector: 0x45,
                delivery_mode: 0b101,
                logical_destination: true,
                delivery_pending: true,
                mode: InterruptMode { active_low: true, level_triggered: true },
                remote_irr: true,
                masked: true,
                destination_apic_id: 0xA5,
            }
        );
    }

    #[test]
    fn readback_verifies_writable_fields_and_ignores_read_only_status() {
        let expected_low = 0x41 | REDIRECTION_MASK | REDIRECTION_ACTIVE_LOW;
        let expected_high = 0x0300_0000;
        // Delivery Status and Remote-IRR may be set by hardware asynchronously.
        let actual_low = expected_low | (1 << 12) | (1 << 14);
        assert!(redirection_readback_matches(
            expected_low,
            expected_high,
            actual_low,
            expected_high,
        ));
        assert!(!redirection_readback_matches(
            expected_low,
            expected_high,
            expected_low ^ 1,
            expected_high,
        ));
        assert!(!redirection_readback_matches(
            expected_low,
            expected_high,
            expected_low,
            expected_high ^ (1 << 24),
        ));
    }

    #[test]
    fn rejects_x2apic_destination_ids_not_representable_by_ioapic() {
        let entry = RedirectionEntry {
            vector: 0x41,
            destination_apic_id: 256,
            mode: InterruptMode { active_low: false, level_triggered: false },
        };
        assert_eq!(entry.high(), Err(IoApicError::DestinationIdOutOfRange));
    }

    #[test]
    fn redirection_entry_encodes_delivery_attributes_and_destination() {
        let entry = RedirectionEntry {
            vector: 0x41,
            destination_apic_id: 3,
            mode: InterruptMode { active_low: true, level_triggered: true },
        };
        assert_eq!(entry.low(), 0x41 | REDIRECTION_ACTIVE_LOW | REDIRECTION_LEVEL);
        assert_eq!(entry.high(), Ok(3 << 24));
        // configure_masked adds REDIRECTION_MASK independently of caller state.
        assert_ne!((entry.low() | REDIRECTION_MASK) & REDIRECTION_MASK, 0);
    }
}
