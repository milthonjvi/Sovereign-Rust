use super::EfiStatus;

/// UEFI Simple Text Output Protocol prefix. Reset and OutputString are the
/// first two function slots in the firmware ABI.
#[repr(C)]
pub struct EfiSimpleTextOutputProtocol {
    reset: usize,
    output_string: Option<unsafe extern "efiapi" fn(
        this: *mut EfiSimpleTextOutputProtocol,
        string: *const u16,
    ) -> EfiStatus>,
}

/// Narrow console wrapper for firmware-provided UTF-16 text output.
pub struct Console<'a> {
    protocol: &'a mut EfiSimpleTextOutputProtocol,
}

impl<'a> Console<'a> {
    /// Construct a console wrapper from the firmware protocol pointer.
    ///
    /// # Safety
    /// The pointer must reference a valid UEFI protocol instance that remains
    /// valid for lifetime 'a. The caller must ensure exclusive use.
    pub unsafe fn from_ptr(protocol: *mut EfiSimpleTextOutputProtocol) -> Self {
        // SAFETY: The caller guarantees validity and exclusive access.
        Self {
            protocol: unsafe { &mut *protocol },
        }
    }

    /// Write ASCII as null-terminated UTF-16 without allocating.
    pub fn write_ascii(&mut self, text: &str) -> bool {
        let Some(output_string) = self.protocol.output_string else {
            return false;
        };

        let bytes = text.as_bytes();
        let mut offset = 0;

        while offset < bytes.len() {
            let mut buffer = [0u16; 128];
            let count = core::cmp::min(buffer.len() - 1, bytes.len() - offset);

            for (slot, byte) in buffer.iter_mut().zip(&bytes[offset..offset + count]) {
                *slot = if byte.is_ascii() { *byte as u16 } else { b'?' as u16 };
            }

            // SAFETY: The stack buffer is valid, UTF-16, and null-terminated.
            let status = unsafe { output_string(self.protocol, buffer.as_ptr()) };
            if status != 0 {
                return false;
            }

            offset += count;
        }

        true
    }
}
