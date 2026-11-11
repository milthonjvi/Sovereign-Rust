//! Firmware ACPI Control Structure (FACS).
//!
//! Unlike an SDT, FACS has no checksum field and must not be parsed with
//! Sdt::parse. The caller is responsible for mapping the complete physical
//! range before passing the borrowed bytes to this parser.

use super::AcpiError;

pub const FACS_SIGNATURE: [u8; 4] = *b"FACS";
pub const FACS_MIN_LENGTH: usize = 64;
pub const FACS_ALIGNMENT: u64 = 64;
const FLAG_S4_BIOS: u32 = 1 << 0;
const FLAG_64BIT_WAKE: u32 = 1 << 1;
const OSPM_FLAG_64BIT_WAKE: u32 = 1 << 0;
const GLOBAL_LOCK_PENDING: u32 = 1 << 0;
const GLOBAL_LOCK_OWNED: u32 = 1 << 1;


/// Result of applying the ACPI global-lock acquire transition to a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlobalLockAcquire {
    /// Updated lock word; publish it with atomic compare-exchange, never a plain store.
    pub updated_word: u32,
    pub acquired: bool,
}

/// Result of applying the ACPI global-lock release transition to a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlobalLockRelease {
    /// Updated lock word; publish it atomically.
    pub updated_word: u32,
    /// A pending waiter exists and requires the ACPI Global Lock Release notification.
    pub pending_waiter: bool,
}

/// Pure ACPI global-lock state transitions.
///
/// These calculate the next word from an observed value. The shared FACS
/// field still requires an atomic compare-exchange retry loop.
pub struct GlobalLock;

impl GlobalLock {
    pub const PENDING: u32 = GLOBAL_LOCK_PENDING;
    pub const OWNED: u32 = GLOBAL_LOCK_OWNED;

    pub const fn is_pending(word: u32) -> bool {
        word & GLOBAL_LOCK_PENDING != 0
    }

    pub const fn is_owned(word: u32) -> bool {
        word & GLOBAL_LOCK_OWNED != 0
    }

    /// Calculate one acquire attempt while preserving reserved bits.
    pub const fn acquire_transition(observed: u32) -> GlobalLockAcquire {
        // ACPI 6.6 §5.2.10.1: clear a stale pending bit before
        // attempting ownership; if already owned, set pending again.
        let without_pending = observed & !GLOBAL_LOCK_PENDING;
        if without_pending & GLOBAL_LOCK_OWNED == 0 {
            GlobalLockAcquire {
                updated_word: without_pending | GLOBAL_LOCK_OWNED,
                acquired: true,
            }
        } else {
            GlobalLockAcquire {
                updated_word: without_pending | GLOBAL_LOCK_PENDING,
                acquired: false,
            }
        }
    }

    /// Calculate one release transition. A pending waiter requires notification.
    pub const fn release_transition(observed: u32) -> GlobalLockRelease {
        if observed & GLOBAL_LOCK_PENDING != 0 {
            GlobalLockRelease {
                updated_word: observed & !(GLOBAL_LOCK_PENDING | GLOBAL_LOCK_OWNED),
                pending_waiter: true,
            }
        } else {
            GlobalLockRelease {
                updated_word: observed & !GLOBAL_LOCK_OWNED,
                pending_waiter: false,
            }
        }
    }
}


/// Selected firmware waking vector, retaining its address width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WakingVector {
    Legacy32(u32),
    Extended64(u64),
}

/// Validated, read-only FACS metadata.
///
/// Firmware-owned state is exposed as values. Mutating the global lock or
/// waking vectors requires a separately synchronized architecture layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Facs {
    pub physical_address: u64,
    pub length: u32,
    pub hardware_signature: u32,
    pub firmware_waking_vector: u32,
    pub global_lock: u32,
    pub flags: u32,
    pub x_firmware_waking_vector: u64,
    pub version: u8,
    pub ospm_flags: u32,
}

impl Facs {
    /// Parse a FACS at its physical address. FACS has no SDT checksum.
    pub fn parse(physical_address: u64, bytes: &[u8]) -> Result<Self, AcpiError> {
        if physical_address == 0 {
            return Err(AcpiError::UnsupportedAddress);
        }
        if physical_address % FACS_ALIGNMENT != 0 {
            return Err(AcpiError::MisalignedTable);
        }
        if bytes.len() < 8 {
            return Err(AcpiError::BufferTooSmall);
        }
        if bytes.get(0..4) != Some(&FACS_SIGNATURE) {
            return Err(AcpiError::InvalidSignature);
        }

        let length = read_u32(bytes, 4)?;
        let length_usize = usize::try_from(length).map_err(|_| AcpiError::InvalidLength)?;
        if length_usize < FACS_MIN_LENGTH || length_usize > bytes.len() {
            return Err(AcpiError::InvalidLength);
        }

        let data = &bytes[..length_usize];
        Ok(Self {
            physical_address,
            length,
            hardware_signature: read_u32(data, 8)?,
            firmware_waking_vector: read_u32(data, 12)?,
            global_lock: read_u32(data, 16)?,
            flags: read_u32(data, 20)?,
            x_firmware_waking_vector: read_u64(data, 24)?,
            version: data[32],
            ospm_flags: read_u32(data, 36)?,
        })
    }

    /// Calculate an acquire attempt from the parsed FACS lock snapshot.
    ///
    /// This is only a pure state transition, not a lock operation. The caller
    /// must first verify Fadt::global_lock_supported(); Hardware Reduced ACPI
    /// explicitly prohibits the Global Lock even if a FACS happens to exist.
    /// The result must be committed to firmware memory with atomic CAS.
    pub const fn global_lock_acquire_transition(&self) -> GlobalLockAcquire {
        GlobalLock::acquire_transition(self.global_lock)
    }

    /// Calculate release from the parsed FACS lock snapshot.
    ///
    /// This is only a pure state transition, not a lock operation. The caller
    /// must first verify Fadt::global_lock_supported().
    /// The result must be committed to firmware memory with atomic CAS.
    pub const fn global_lock_release_transition(&self) -> GlobalLockRelease {
        GlobalLock::release_transition(self.global_lock)
    }

    pub const fn supports_s4_bios(&self) -> bool {
        self.flags & FLAG_S4_BIOS != 0
    }

    /// Whether firmware supports a 64-bit execution environment for wake.
    pub const fn supports_64bit_wake(&self) -> bool {
        self.flags & FLAG_64BIT_WAKE != 0
    }

    /// Whether OSPM requests a 64-bit execution environment for the X vector.
    pub const fn requests_64bit_wake(&self) -> bool {
        self.ospm_flags & OSPM_FLAG_64BIT_WAKE != 0
    }

    /// Whether both sides agree to use the 64-bit wake execution environment.
    pub const fn uses_64bit_wake_environment(&self) -> bool {
        self.supports_64bit_wake() && self.requests_64bit_wake()
    }

    pub const fn has_64bit_waking_vector(&self) -> bool {
        self.x_firmware_waking_vector != 0
    }

    /// Select the waking vector using the firmware-defined precedence.
    ///
    /// Firmware checks X_Firmware_Waking_Vector first whenever it is nonzero;
    /// 64BIT_WAKE_SUPPORTED_F controls the execution environment, not whether
    /// the extended address field takes precedence over the legacy field.
    /// This only selects metadata; it does not install a trampoline or sleep.
    pub const fn waking_vector(&self) -> Option<WakingVector> {
        if self.x_firmware_waking_vector != 0 {
            Some(WakingVector::Extended64(self.x_firmware_waking_vector))
        } else if self.firmware_waking_vector != 0 {
            Some(WakingVector::Legacy32(self.firmware_waking_vector))
        } else {
            None
        }
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    let data = bytes.get(offset..offset + 4).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, AcpiError> {
    let data = bytes.get(offset..offset + 8).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]))
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::vec::Vec<u8> {
        let mut bytes = std::vec![0u8; FACS_MIN_LENGTH];
        bytes[0..4].copy_from_slice(b"FACS");
        bytes[4..8].copy_from_slice(&(FACS_MIN_LENGTH as u32).to_le_bytes());
        bytes[8..12].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&0x1000u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&0xA5A5u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&(FLAG_S4_BIOS | FLAG_64BIT_WAKE).to_le_bytes());
        bytes[24..32].copy_from_slice(&0x1_0000_1000u64.to_le_bytes());
        bytes[32] = 2;
        bytes[36..40].copy_from_slice(&1u32.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_facs_without_sdt_checksum() {
        let mut bytes = fixture();
        bytes[9] = 0xFF;
        let facs = Facs::parse(0x4000, &bytes).unwrap();
        assert_eq!(facs.hardware_signature, 0x1234_5678);
        assert_eq!(facs.firmware_waking_vector, 0x1000);
        assert_eq!(facs.x_firmware_waking_vector, 0x1_0000_1000);
        assert!(facs.supports_s4_bios());
        assert!(facs.supports_64bit_wake());
        assert!(facs.has_64bit_waking_vector());
    }



    #[test]
    fn extended_waking_vector_has_precedence_independent_of_execution_mode() {
        let mut bytes = fixture();
        let facs = Facs::parse(0x4000, &bytes).unwrap();
        assert_eq!(facs.waking_vector(), Some(WakingVector::Extended64(0x1_0000_1000)));
        assert!(facs.uses_64bit_wake_environment());

        // The X vector still has precedence when firmware only supports the
        // 32-bit wake environment. The support flag controls execution mode.
        bytes[20..24].copy_from_slice(&FLAG_S4_BIOS.to_le_bytes());
        let facs = Facs::parse(0x4000, &bytes).unwrap();
        assert_eq!(facs.waking_vector(), Some(WakingVector::Extended64(0x1_0000_1000)));
        assert!(!facs.uses_64bit_wake_environment());

        // Without an X vector, firmware falls back to the legacy field.
        bytes[24..32].copy_from_slice(&0u64.to_le_bytes());
        let facs = Facs::parse(0x4000, &bytes).unwrap();
        assert_eq!(facs.waking_vector(), Some(WakingVector::Legacy32(0x1000)));
    }

    #[test]
    fn returns_no_waking_vector_when_firmware_provides_none() {
        let mut bytes = fixture();
        bytes[12..16].copy_from_slice(&0u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&0u64.to_le_bytes());
        let facs = Facs::parse(0x4000, &bytes).unwrap();
        assert_eq!(facs.waking_vector(), None);
    }

    #[test]
    fn global_lock_acquire_sets_owned_or_pending_and_preserves_reserved_bits() {
        let free = 0x8000_0000;
        let acquired = GlobalLock::acquire_transition(free);
        assert_eq!(acquired.updated_word, free | GlobalLock::OWNED);
        assert!(acquired.acquired);

        // A stale pending bit is cleared before a fresh ownership attempt.
        let stale_pending = free | GlobalLock::PENDING;
        let acquired = GlobalLock::acquire_transition(stale_pending);
        assert_eq!(acquired.updated_word, free | GlobalLock::OWNED);
        assert!(acquired.acquired);

        let busy = 0x4000_0000 | GlobalLock::OWNED;
        let pending = GlobalLock::acquire_transition(busy);
        assert_eq!(pending.updated_word, busy | GlobalLock::PENDING);
        assert!(!pending.acquired);
    }

    #[test]
    fn global_lock_release_clears_pending_and_owned_and_reports_waiter() {
        let with_waiter = 0x8000_0000 | GlobalLock::OWNED | GlobalLock::PENDING;
        let released = GlobalLock::release_transition(with_waiter);
        assert_eq!(released.updated_word, 0x8000_0000);
        assert!(released.pending_waiter);

        let without_waiter = 0x4000_0000 | GlobalLock::OWNED;
        let released = GlobalLock::release_transition(without_waiter);
        assert_eq!(released.updated_word, 0x4000_0000);
        assert!(!released.pending_waiter);
    }

    #[test]
    fn rejects_bad_signature_alignment_and_length() {
        let bytes = fixture();
        assert_eq!(Facs::parse(0x4001, &bytes), Err(AcpiError::MisalignedTable));
        assert_eq!(Facs::parse(0, &bytes), Err(AcpiError::UnsupportedAddress));
        let mut bad_signature = bytes.clone();
        bad_signature[0..4].copy_from_slice(b"BAD!");
        assert_eq!(Facs::parse(0x4000, &bad_signature), Err(AcpiError::InvalidSignature));
        let mut short = bytes;
        short[4..8].copy_from_slice(&32u32.to_le_bytes());
        assert_eq!(Facs::parse(0x4000, &short), Err(AcpiError::InvalidLength));
    }

    #[test]
    fn rejects_truncated_mapping() {
        let bytes = fixture();
        assert_eq!(Facs::parse(0x4000, &bytes[..48]), Err(AcpiError::InvalidLength));
    }
}
