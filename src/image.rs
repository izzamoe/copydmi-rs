//! Turning a loaded BIOS buffer into the real, flash-ready firmware image.
//!
//! CLEAN-HEADER STRIPPING (the "clean BIOS" step): the .cap firmware image extracted
//! from Lenovo's official installer for this chip family (e.g. e8cn39ww.exe) is NOT
//! the raw flash-ready image — it's wrapped in a 792-byte (0x318) header, and the real
//! firmware only starts after that. Verified byte-for-byte against TWO real BIOS
//! versions (e8cn39ww.exe AND e8cn41ww.exe — identical header offset in both):
//!   - Extracted BIOS.cap size: 8,950,768 bytes total
//!   - Bytes [0x000, 0x050): EFI_CAPSULE_HEADER (80 bytes: GUID, HeaderSize=0x50,
//!     Flags, CapsuleImageSize=8,950,768 i.e. the WHOLE wrapped file)
//!   - Bytes [0x050, 0x098): an outer Firmware Volume header (72 bytes, GUID
//!     78e58c8c-3d8a-1c4f-9935-8961-85c32dd3) whose declared FvLength spans nearly
//!     the entire file — this FV *wraps* the real firmware as a single big FFS file,
//!     it is not the real firmware layout itself.
//!   - Bytes [0x098, 0x318): FFS file header + signature/crypto blob (640 bytes)
//!   - Bytes [0x318, 0x800318): THE REAL FIRMWARE IMAGE — exactly 8,388,608 bytes
//!     (8 MiB / 0x800000), matching the Winbond W25Q64-class chip on this laptop
//!     family. Confirmed by: (a) every _FVH-validated Firmware Volume inside this
//!     slice lands on a round address (0x310000, 0x360000, 0x390000, 0x3A0000,
//!     0x730000) and the chain is perfectly contiguous, ending EXACTLY at 0x800000;
//!     (b) the SMBIOS/DMI vendor string "LENV\0" sits at RELATIVE offset 0x1000 and
//!     0x2000 inside this slice — i.e. right at the start and middle of the
//!     community-documented DMI range 0x1000-0x2FFF; (c) identical header offset in
//!     both e8cn39ww.exe and e8cn41ww.exe.
//!   - Bytes [0x800318, end): installer/debug metadata tail (562,152 bytes) — NOT
//!     firmware, safe to discard (PDB paths, printf-style debug format strings).
//!
//! By default, copydmi strips this 792-byte header from --old/--new BEFORE doing
//! the DMI transplant (which uses offsets relative to the real firmware, as per the
//! community guide), so --out is a ready-to-flash 8 MiB raw image, not a
//! capsule-wrapped blob. Override with --header-size/--old-header-size/--chip-size
//! or disable entirely with --no-trim if your model's wrapper differs.

/// Remove `header_size` bytes of capsule/wrapper header from the front of
/// `buf`. `label` ("old"/"new") is only used in the error message.
pub fn strip_header(buf: &mut Vec<u8>, header_size: usize, label: &str) -> Result<(), String> {
    if buf.len() <= header_size {
        return Err(format!(
            "--{label} is only {} bytes, too small to strip its header-size (0x{:X} / {} bytes) \
             from the front. Pass --no-trim or a smaller header-size if this file doesn't \
             have this wrapper.",
            buf.len(),
            header_size,
            header_size
        ));
    }
    buf.drain(..header_size);
    Ok(())
}

/// What [`trim_to_chip_size`] did to a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trim {
    /// Tail metadata past the chip size was cut off; holds the original length.
    Truncated { original_len: usize },
    /// Buffer is smaller than the chip size; left as-is (never padded).
    Short,
    /// Buffer already has exactly the chip size.
    Exact,
}

/// Truncate `buf` down to `chip_size` bytes. Only ever removes bytes from
/// the end — the DMI region and firmware volumes live near the start.
pub fn trim_to_chip_size(buf: &mut Vec<u8>, chip_size: usize) -> Trim {
    use std::cmp::Ordering;
    match buf.len().cmp(&chip_size) {
        Ordering::Greater => {
            let original_len = buf.len();
            buf.truncate(chip_size);
            Trim::Truncated { original_len }
        }
        Ordering::Less => Trim::Short,
        Ordering::Equal => Trim::Exact,
    }
}

/// Ensure `buf` is large enough to contain the inclusive DMI range ending at `end`.
pub fn ensure_covers(buf: &[u8], end: usize, which: &str) -> Result<(), String> {
    if buf.len() <= end {
        return Err(format!(
            "{which} bios file is too small ({} bytes) for DMI range ending at 0x{:X}",
            buf.len(),
            end
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_removes_exactly_the_front_bytes() {
        let mut buf: Vec<u8> = (0..=255).collect();
        strip_header(&mut buf, 0x18, "new").unwrap();
        assert_eq!(buf.len(), 256 - 0x18);
        assert_eq!(buf[0], 0x18);
        assert_eq!(*buf.last().unwrap(), 255);
    }

    #[test]
    fn strip_zero_is_a_noop() {
        let mut buf = vec![1, 2, 3];
        strip_header(&mut buf, 0, "old").unwrap();
        assert_eq!(buf, [1, 2, 3]);
    }

    #[test]
    fn strip_refuses_to_consume_whole_buffer() {
        let mut buf = vec![0u8; 0x318];
        let err = strip_header(&mut buf, 0x318, "old").unwrap_err();
        assert_eq!(
            err,
            "--old is only 792 bytes, too small to strip its header-size (0x318 / 792 bytes) \
             from the front. Pass --no-trim or a smaller header-size if this file doesn't \
             have this wrapper."
        );
        assert_eq!(buf.len(), 0x318, "buffer must be untouched on error");
    }

    #[test]
    fn trim_outcomes() {
        let mut buf = vec![7u8; 10];
        assert_eq!(
            trim_to_chip_size(&mut buf, 4),
            Trim::Truncated { original_len: 10 }
        );
        assert_eq!(buf, [7; 4]);
        assert_eq!(trim_to_chip_size(&mut buf, 4), Trim::Exact);
        assert_eq!(trim_to_chip_size(&mut buf, 8), Trim::Short);
        assert_eq!(buf.len(), 4, "short buffers are never padded");
    }

    #[test]
    fn ensure_covers_requires_end_inside_buffer() {
        let buf = vec![0u8; 0x3000];
        assert!(ensure_covers(&buf, 0x2FFF, "old").is_ok());
        assert_eq!(
            ensure_covers(&buf, 0x3000, "new"),
            Err("new bios file is too small (12288 bytes) for DMI range ending at 0x3000".into())
        );
    }
}
