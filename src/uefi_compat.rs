//! Minimal C ABI shims required by the UEFI crate on the PE/COFF target.

/// Returns the length of a NUL-terminated UTF-16 string.
///
/// UEFI supplies valid firmware strings to its protocol helpers.  Keeping the
/// shim in the kernel makes every release configuration link, not only the
/// input-smoke configuration.
#[cfg(feature = "uefi-bin")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcslen(mut string: *const u16) -> usize {
    let mut length = 0;
    // SAFETY: matches C `wcslen`; the caller owns a valid NUL-terminated UTF-16
    // string and this function never writes through the pointer.
    unsafe {
        while *string != 0 {
            length += 1;
            string = string.add(1);
        }
    }
    length
}
