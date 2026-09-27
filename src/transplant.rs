//! The DMI transplant itself: comparing and copying an inclusive byte range
//! from the old image into the new one, plus the hex preview shown to the user.

/// Number of positions at which `a` and `b` differ (compared up to the shorter length).
pub fn count_differing(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

/// Return a copy of `new` with bytes `[start, end]` (inclusive) replaced by
/// the same range from `old`. Both buffers must contain the range.
pub fn transplant(old: &[u8], new: &[u8], start: usize, end: usize) -> Vec<u8> {
    let mut patched = new.to_vec();
    patched[start..=end].copy_from_slice(&old[start..=end]);
    patched
}

/// One preview line: `  {label} @0x{start}: XX XX ...` showing up to
/// `len_show` bytes from `start` (clamped to the buffer end).
pub fn hex_preview_line(label: &str, buf: &[u8], start: usize, len_show: usize) -> String {
    let end = (start + len_show).min(buf.len());
    let mut line = format!("  {label} @0x{start:06X}: ");
    for b in &buf[start..end] {
        line.push_str(&format!("{b:02X} "));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_differences() {
        assert_eq!(count_differing(&[1, 2, 3], &[1, 2, 3]), 0);
        assert_eq!(count_differing(&[1, 2, 3], &[0, 2, 4]), 2);
        assert_eq!(count_differing(&[], &[]), 0);
    }

    #[test]
    fn transplant_copies_inclusive_range_only() {
        let old = vec![0xAAu8; 8];
        let new = vec![0x11u8; 8];
        let patched = transplant(&old, &new, 2, 4);
        assert_eq!(patched, [0x11, 0x11, 0xAA, 0xAA, 0xAA, 0x11, 0x11, 0x11]);
        assert_eq!(new, [0x11; 8], "input must not be modified");
    }

    #[test]
    fn transplant_single_byte_range() {
        assert_eq!(transplant(&[9, 9], &[0, 0], 1, 1), [0, 9]);
    }

    #[test]
    fn preview_formats_and_clamps() {
        let buf = [0x00, 0x1F, 0xAB, 0xFF];
        assert_eq!(
            hex_preview_line("old", &buf, 1, 2),
            "  old @0x000001: 1F AB "
        );
        assert_eq!(
            hex_preview_line("new (before patch)", &buf, 2, 32),
            "  new (before patch) @0x000002: AB FF "
        );
        assert_eq!(hex_preview_line("out", &buf, 4, 32), "  out @0x000004: ");
    }
}
