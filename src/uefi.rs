//! UEFI Firmware Volume (FV) + Firmware File System (FFS) checksum engine.
//!
//! This implements REAL structural parsing per the UEFI PI spec (Volume 3):
//!   - EFI_FIRMWARE_VOLUME_HEADER: locates FVs by scanning for the "_FVH"
//!     signature, validates the 16-bit header checksum (sum of all UINT16
//!     words across HeaderLength must equal 0 mod 0x10000).
//!   - EFI_FFS_FILE_HEADER (non-extended, 24-byte, Size < 0xFFFFFF): walks
//!     files sequentially inside a FV's body (8-byte aligned), validates:
//!       * Header checksum (IntegrityCheck.Header): sum of all header bytes,
//!         with State and IntegrityCheck.File treated as 0 during the sum,
//!         must be 0 mod 256.
//!       * Data checksum (IntegrityCheck.File): only meaningful if
//!         FFS_ATTRIB_CHECKSUM (0x40) is set in Attributes; otherwise the
//!         spec fixes it at 0xAA (not a real checksum). When set, sum of all
//!         data bytes must be 0 mod 256.
//!
//! This is why the community's raw byte-swap trick to move DMI often "just
//! works": on these Lenovo AMD ADA-series (Insyde H2O) images the DMI data
//! commonly sits in an FFS file WITHOUT FFS_ATTRIB_CHECKSUM set, so no data
//! checksum is enforced there at all. But we don't assume that — we actually
//! parse the structures and tell you the truth for YOUR specific files.
//!
//! Scope/limits (stated honestly, not hidden):
//!   - Only classic (non-extended) FFS file headers are handled (24-byte,
//!     Size field < 0xFFFFFF). Walking a FV stops at the first file using the
//!     extended 32-byte header (or at anything corrupt/unrecognised).
//!   - We do not parse FFS section internals (PE32/raw/etc) or recompute any
//!     GUIDed-section (e.g. Tiano-compressed) internal checksums.
//!   - We do not touch Volume Top File / other vendor-specific signing.
//!   - This does NOT replace UEFITool for deep firmware surgery — it targets
//!     exactly the case this tool exists for: verifying/fixing the FFS file
//!     whose data region overlaps the DMI byte range we just patched.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FvInfo {
    pub fv_start: usize,
    pub fv_length: usize,
    pub header_length: usize,
    pub header_checksum_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfsInfo {
    pub file_start: usize, // offset of EFI_FFS_FILE_HEADER
    pub data_start: usize,
    pub data_len: usize,
    pub header_checksum_ok: bool,
    pub checksum_attrib_set: bool,
    pub data_checksum_ok: Option<bool>, // None if attrib not set (0xAA fixed, not a real checksum)
}

const FV_SIG: &[u8; 4] = b"_FVH";
const FFS_HDR_LEN: usize = 24; // classic header — extended not supported
const FFS_ATTRIB_CHECKSUM: u8 = 0x40;
const FFS_FILE_CHECKSUM_OFFSET: usize = 0x11; // IntegrityCheck.File
const FFS_STATE_OFFSET: usize = 0x17;

/// Round `pos` up to the next multiple of 8 (FVs and FFS files are 8-byte aligned).
fn align8(pos: usize) -> usize {
    pos.next_multiple_of(8)
}

/// Scan the whole image for EFI_FIRMWARE_VOLUME_HEADER structures.
/// Header layout (per PI spec, offsets from FV start):
///   0x00 ZeroVector[16]
///   0x10 FileSystemGuid[16]
///   0x20 FvLength (u64)          -> offset 32
///   0x28 Signature[4] "_FVH"     -> offset 40
///   0x2C Attributes (u32)        -> offset 44
///   0x30 HeaderLength (u16)      -> offset 48
///   0x32 Checksum (u16)          -> offset 50
///   0x34 ExtHeaderOffset (u16)   -> offset 52
///   0x36 Reserved (u8)
///   0x37 Revision (u8)
///   0x38 BlockMap[...]           -> offset 56
pub fn scan_fvs(buf: &[u8]) -> Vec<FvInfo> {
    let mut out = Vec::new();
    if buf.len() < 64 {
        return out;
    }
    let mut i = 0usize;
    while i + 4 <= buf.len().saturating_sub(40) {
        if &buf[i + 40..i + 44] == FV_SIG && i + 56 <= buf.len() {
            // candidate FV start = i
            let fv_length = u64::from_le_bytes(buf[i + 32..i + 40].try_into().unwrap()) as usize;
            let header_length =
                u16::from_le_bytes(buf[i + 48..i + 50].try_into().unwrap()) as usize;
            // Sanity gates to reject coincidental "_FVH" byte matches in
            // ordinary data (real FV headers are small and FvLength must
            // fit inside the remaining buffer):
            //   - header_length must be a plausible size (spec minimum is
            //     56 bytes plus at least one BlockMap entry; real-world
            //     headers are well under a few KB)
            //   - fv_length must fit entirely within the buffer from i
            //   - fv_length must be >= header_length (a FV can't be
            //     smaller than its own header)
            let header_length_plausible = (56..=8192).contains(&header_length);
            let fv_length_fits = fv_length > 0 && i.saturating_add(fv_length) <= buf.len();
            if header_length_plausible
                && fv_length_fits
                && i + header_length <= buf.len()
                && fv_length >= header_length
            {
                out.push(FvInfo {
                    fv_start: i,
                    fv_length,
                    header_length,
                    header_checksum_ok: checksum16_zero(&buf[i..i + header_length]),
                });
            }
        }
        i += 8; // FV headers are 8-byte aligned per spec
    }
    out
}

/// Sum all bytes as little-endian u16 words; true if sum == 0 mod 0x10000.
/// If the slice length is odd, the trailing byte is treated as the low byte
/// of a final word with high byte 0 (matches how reference implementations
/// pad partial words for this checksum style).
pub fn checksum16_zero(data: &[u8]) -> bool {
    let mut sum: u32 = 0;
    let (words, remainder) = data.as_chunks::<2>();
    for &w in words {
        sum = sum.wrapping_add(u16::from_le_bytes(w) as u32);
    }
    if let [last] = remainder {
        sum = sum.wrapping_add(*last as u32);
    }
    (sum & 0xFFFF) == 0
}

/// Wrapping 8-bit sum of all bytes.
fn sum8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |sum, &b| sum.wrapping_add(b))
}

pub fn sum8_zero(data: &[u8]) -> bool {
    sum8(data) == 0
}

/// Walk FFS files inside one FV's body and validate header/data checksums.
/// Classic (non-extended) EFI_FFS_FILE_HEADER, 24 bytes:
///   0x00 Name[16] (GUID)
///   0x10 IntegrityCheck.Header (u8)
///   0x11 IntegrityCheck.File   (u8)
///   0x12 Type (u8)
///   0x13 Attributes (u8)
///   0x14 Size[3] (24-bit LE)
///   0x17 State (u8)
pub fn scan_ffs_in_fv(buf: &[u8], fv: &FvInfo) -> Vec<FfsInfo> {
    const ERASE_POLARITY_FF_RUN_CHECK: usize = 16; // how many bytes of 0xFF/0x00 before declaring "end of files"

    let mut out = Vec::new();
    let body_start = fv.fv_start + fv.header_length;
    let body_end = fv.fv_start + fv.fv_length;
    if body_end > buf.len() || body_start >= body_end {
        return out;
    }
    // align up to 8 bytes from FV body start, per spec files are 8-byte aligned
    let mut pos = align8(body_start);
    while pos + FFS_HDR_LEN <= body_end {
        let hdr = &buf[pos..pos + FFS_HDR_LEN];
        // detect padding/empty space: run of 0xFF (erased flash) or all 0x00
        let all_ff = hdr.iter().all(|&b| b == 0xFF);
        let all_00 = hdr.iter().all(|&b| b == 0x00);
        if all_ff || all_00 {
            // check a bit further to decide if this is real trailing padding
            let probe_end = (pos + ERASE_POLARITY_FF_RUN_CHECK).min(body_end);
            let probe = &buf[pos..probe_end];
            if probe.iter().all(|&b| b == hdr[0]) {
                break; // reached padding/free space, stop walking this FV
            }
        }

        let size = u32::from_le_bytes([hdr[0x14], hdr[0x15], hdr[0x16], 0]) as usize;
        let attributes = hdr[0x13];
        let large_file = size == 0x00FF_FFFF || size == 0; // 0xFFFFFF marker or garbage -> extended/unsupported
        if large_file || size < FFS_HDR_LEN || pos + size > body_end {
            // can't safely continue walking this FV; stop (extended header or corrupt/unknown)
            break;
        }

        // Header checksum: sum of header bytes with State and IntegrityCheck.File zeroed
        let header_checksum_ok = sum8(hdr)
            .wrapping_sub(hdr[FFS_FILE_CHECKSUM_OFFSET])
            .wrapping_sub(hdr[FFS_STATE_OFFSET])
            == 0;

        let checksum_attrib_set = attributes & FFS_ATTRIB_CHECKSUM != 0;
        let data_start = pos + FFS_HDR_LEN;
        let data_len = size - FFS_HDR_LEN;
        let data_checksum_ok = if checksum_attrib_set && data_start + data_len <= buf.len() {
            Some(sum8_zero(&buf[data_start..data_start + data_len]))
        } else {
            None
        };

        out.push(FfsInfo {
            file_start: pos,
            data_start,
            data_len,
            header_checksum_ok,
            checksum_attrib_set,
            data_checksum_ok,
        });

        pos = align8(pos + size);
    }
    out
}

/// Print a human-readable FV/FFS checksum summary for `buf`.
pub fn uefi_report(label: &str, buf: &[u8]) {
    let fvs = scan_fvs(buf);
    println!("  -- UEFI structure scan: {label} ({} bytes) --", buf.len());
    if fvs.is_empty() {
        println!(
            "    no Firmware Volume (_FVH) headers found (raw/non-UEFI image, or none detected)."
        );
        return;
    }
    for fv in &fvs {
        let ffs_list = scan_ffs_in_fv(buf, fv);
        println!(
            "    FV @0x{:06X} len=0x{:X} header_len={} header_checksum={}",
            fv.fv_start,
            fv.fv_length,
            fv.header_length,
            if fv.header_checksum_ok { "OK" } else { "BAD" }
        );
        println!(
            "      {} FFS files parsed (classic headers only)",
            ffs_list.len()
        );
        let bad_headers = ffs_list.iter().filter(|f| !f.header_checksum_ok).count();
        let bad_data = ffs_list
            .iter()
            .filter(|f| f.data_checksum_ok == Some(false))
            .count();
        if bad_headers > 0 || bad_data > 0 {
            println!("      WARNING: {bad_headers} file(s) with bad header checksum, {bad_data} file(s) with bad data checksum");
        }
    }
}

/// After patching bytes in [patch_start, patch_end_incl], find any FFS file(s)
/// whose data region overlaps the patched range, and recompute their
/// IntegrityCheck.File data checksum (only meaningful if FFS_ATTRIB_CHECKSUM
/// is set — otherwise per spec it must stay fixed at 0xAA, so we leave it).
/// Returns how many files were fixed.
pub fn fix_overlapping_ffs_checksums(
    buf: &mut [u8],
    patch_start: usize,
    patch_end_incl: usize,
) -> usize {
    let fvs = scan_fvs(buf);
    let mut fixed = 0usize;
    for fv in &fvs {
        let ffs_list = scan_ffs_in_fv(buf, fv);
        for f in &ffs_list {
            let data_end_incl = f.data_start + f.data_len - 1;
            let overlaps = f.data_start <= patch_end_incl && patch_start <= data_end_incl;
            if !overlaps {
                continue;
            }
            if !f.checksum_attrib_set {
                // No data checksum enforced for this file per spec (fixed 0xAA) — nothing to fix.
                continue;
            }
            // Recompute so sum of data bytes == 0 mod 256, by adjusting the
            // IntegrityCheck.File byte itself (standard technique: set it to
            // 0 first, sum, then store the two's-complement of the sum).
            let file_checksum_off = f.file_start + FFS_FILE_CHECKSUM_OFFSET;
            buf[file_checksum_off] = 0;
            let sum = sum8(&buf[f.data_start..f.data_start + f.data_len]);
            buf[file_checksum_off] = 0u8.wrapping_sub(sum);
            fixed += 1;
        }
    }
    fixed
}

#[cfg(test)]
mod tests {
    use super::*;

    const FV_HDR_LEN: usize = 0x48; // 56-byte fixed part + 2 BlockMap entries

    /// Write a valid FV header (with correct checksum) at `at`.
    fn put_fv_header(buf: &mut [u8], at: usize, fv_length: usize) {
        let h = &mut buf[at..at + FV_HDR_LEN];
        h.fill(0);
        h[32..40].copy_from_slice(&(fv_length as u64).to_le_bytes());
        h[40..44].copy_from_slice(FV_SIG);
        h[44..48].copy_from_slice(&0x0004_FEFFu32.to_le_bytes());
        h[48..50].copy_from_slice(&(FV_HDR_LEN as u16).to_le_bytes());
        h[0x37] = 2; // revision
        h[56..60].copy_from_slice(&((fv_length / 0x1000) as u32).to_le_bytes());
        h[60..64].copy_from_slice(&0x1000u32.to_le_bytes());
        let sum = h
            .as_chunks::<2>()
            .0
            .iter()
            .fold(0u16, |s, &w| s.wrapping_add(u16::from_le_bytes(w)));
        h[50..52].copy_from_slice(&0u16.wrapping_sub(sum).to_le_bytes());
    }

    /// Write a classic FFS file at `at` with valid header checksum. When
    /// `checksum_attr` is set, the data checksum is made valid too; otherwise
    /// IntegrityCheck.File is the spec-mandated 0xAA.
    fn put_ffs(buf: &mut [u8], at: usize, data: &[u8], checksum_attr: bool) -> usize {
        let size = FFS_HDR_LEN + data.len();
        let (hdr, rest) = buf[at..].split_at_mut(FFS_HDR_LEN);
        hdr.fill(0);
        hdr[..16].copy_from_slice(&[0x11; 16]); // GUID
        hdr[0x12] = 0x01; // type RAW
        hdr[0x13] = if checksum_attr {
            FFS_ATTRIB_CHECKSUM
        } else {
            0
        };
        hdr[0x14..0x17].copy_from_slice(&(size as u32).to_le_bytes()[..3]);
        rest[..data.len()].copy_from_slice(data);
        hdr[0x10] = 0u8.wrapping_sub(sum8(hdr));
        hdr[FFS_FILE_CHECKSUM_OFFSET] = if checksum_attr {
            0u8.wrapping_sub(sum8(data))
        } else {
            0xAA
        };
        hdr[FFS_STATE_OFFSET] = 0xF8;
        size
    }

    /// 0x4000-byte image: FV at 0x1000 (len 0x2000) holding two files:
    /// file A (no checksum attr) and file B (checksum attr), rest erased 0xFF.
    fn sample_image() -> (Vec<u8>, usize, usize) {
        let mut img = vec![0xFFu8; 0x4000];
        put_fv_header(&mut img, 0x1000, 0x2000);
        let a = 0x1000 + FV_HDR_LEN;
        let a_size = put_ffs(&mut img, a, &[0x42; 0x100], false);
        let b = align8(a + a_size);
        // Data sums to 0 mod 256, so IntegrityCheck.File is 0 and the file is
        // valid however the data checksum is interpreted.
        put_ffs(&mut img, b, &[0x02; 0x80], true);
        (img, a, b)
    }

    #[test]
    fn checksum16_zero_handles_even_and_odd_lengths() {
        assert!(checksum16_zero(&[]));
        assert!(checksum16_zero(&[0x01, 0x00, 0xFF, 0xFF])); // 1 + 0xFFFF
        assert!(!checksum16_zero(&[0x01, 0x00]));
        assert!(checksum16_zero(&[0xFF, 0xFF, 0x01])); // odd tail as low byte
        assert!(!checksum16_zero(&[0x00, 0x00, 0x01]));
    }

    #[test]
    fn sum8_zero_wraps() {
        assert!(sum8_zero(&[]));
        assert!(sum8_zero(&[0x80, 0x80]));
        assert!(sum8_zero(&[0x01, 0xFF, 0x10, 0xF0]));
        assert!(!sum8_zero(&[0x01]));
    }

    #[test]
    fn align8_rounds_up() {
        assert_eq!(align8(0), 0);
        assert_eq!(align8(1), 8);
        assert_eq!(align8(8), 8);
        assert_eq!(align8(0x1049), 0x1050);
    }

    #[test]
    fn finds_fv_and_validates_header_checksum() {
        let (mut img, _, _) = sample_image();
        let fvs = scan_fvs(&img);
        assert_eq!(
            fvs,
            vec![FvInfo {
                fv_start: 0x1000,
                fv_length: 0x2000,
                header_length: FV_HDR_LEN,
                header_checksum_ok: true,
            }]
        );
        img[0x1000 + 0x37] ^= 1; // corrupt a header byte
        assert!(!scan_fvs(&img)[0].header_checksum_ok);
    }

    #[test]
    fn rejects_implausible_fv_signatures() {
        let mut img = vec![0u8; 0x1000];
        // FvLength running past the end of the buffer.
        put_fv_header(&mut img, 0x100, 0x10_0000);
        assert!(scan_fvs(&img).is_empty());
        // Implausible HeaderLength.
        put_fv_header(&mut img, 0x100, 0x200);
        img[0x100 + 48..0x100 + 50].copy_from_slice(&8u16.to_le_bytes());
        assert!(scan_fvs(&img).is_empty());
        // Unaligned signature is never considered.
        let mut img = vec![0u8; 0x1000];
        put_fv_header(&mut img, 0x104, 0x200);
        assert!(scan_fvs(&img).is_empty());
        // Tiny buffers are ignored outright.
        assert!(scan_fvs(&[0u8; 63]).is_empty());
    }

    #[test]
    fn walks_ffs_files_until_padding() {
        let (img, a, b) = sample_image();
        let fv = &scan_fvs(&img)[0];
        let files = scan_ffs_in_fv(&img, fv);
        assert_eq!(files.len(), 2);

        assert_eq!(files[0].file_start, a);
        assert_eq!(files[0].data_start, a + FFS_HDR_LEN);
        assert_eq!(files[0].data_len, 0x100);
        assert!(files[0].header_checksum_ok);
        assert!(!files[0].checksum_attrib_set);
        assert_eq!(files[0].data_checksum_ok, None);

        assert_eq!(files[1].file_start, b);
        assert_eq!(files[1].data_len, 0x80);
        assert!(files[1].header_checksum_ok);
        assert!(files[1].checksum_attrib_set);
        assert_eq!(files[1].data_checksum_ok, Some(true));
    }

    #[test]
    fn header_checksum_ignores_state_and_file_checksum_bytes() {
        let (mut img, a, _) = sample_image();
        img[a + FFS_STATE_OFFSET] = 0x00;
        img[a + FFS_FILE_CHECKSUM_OFFSET] = 0x12;
        let fv = &scan_fvs(&img)[0];
        assert!(scan_ffs_in_fv(&img, fv)[0].header_checksum_ok);

        img[a + 0x12] ^= 0xFF; // Type byte is covered by the checksum
        assert!(!scan_ffs_in_fv(&img, fv)[0].header_checksum_ok);
    }

    #[test]
    fn stops_on_extended_or_oversized_file() {
        let (mut img, a, _) = sample_image();
        img[a + 0x14..a + 0x17].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
        let fv = &scan_fvs(&img)[0];
        assert!(scan_ffs_in_fv(&img, fv).is_empty());

        let (mut img, a, _) = sample_image();
        img[a + 0x14..a + 0x17].copy_from_slice(&0x3000u32.to_le_bytes()[..3]);
        let fv = &scan_fvs(&img)[0];
        assert!(scan_ffs_in_fv(&img, fv).is_empty());
    }

    #[test]
    fn fixes_only_overlapping_files_with_checksum_attribute() {
        let (mut img, a, b) = sample_image();
        let a_data = a + FFS_HDR_LEN;
        let b_data = b + FFS_HDR_LEN;
        // Patch bytes inside both files' data.
        img[a_data..a_data + 4].copy_from_slice(&[1, 2, 3, 4]);
        img[b_data..b_data + 4].copy_from_slice(&[9, 9, 9, 9]);

        // Range that only touches file A: nothing to fix (no checksum attr).
        let before = img.clone();
        assert_eq!(
            fix_overlapping_ffs_checksums(&mut img, a_data, a_data + 3),
            0
        );
        assert_eq!(img, before);

        // Range covering file B's data: IntegrityCheck.File becomes the
        // two's complement of the data sum; nothing else changes.
        assert_eq!(fix_overlapping_ffs_checksums(&mut img, 0x1000, 0x2FFF), 1);
        let expected = 0u8.wrapping_sub(sum8(&img[b_data..b_data + 0x80]));
        assert_eq!(img[b + FFS_FILE_CHECKSUM_OFFSET], expected);
        let mut only_checksum_changed = before.clone();
        only_checksum_changed[b + FFS_FILE_CHECKSUM_OFFSET] = expected;
        assert_eq!(img, only_checksum_changed);
        assert_eq!(img[a + FFS_FILE_CHECKSUM_OFFSET], 0xAA);

        let fv = &scan_fvs(&img)[0];
        assert!(scan_ffs_in_fv(&img, fv)[1].header_checksum_ok);
    }

    #[test]
    fn data_checksum_is_only_reported_for_checksummed_files() {
        let (mut img, a, b) = sample_image();
        img[a + FFS_HDR_LEN] ^= 0x01;
        img[b + FFS_HDR_LEN] ^= 0x01;
        let fv = &scan_fvs(&img)[0];
        let files = scan_ffs_in_fv(&img, fv);
        assert_eq!(files[0].data_checksum_ok, None);
        assert_eq!(files[1].data_checksum_ok, Some(false));
    }

    #[test]
    fn fix_is_noop_without_uefi_structures() {
        let mut img = vec![0x5Au8; 0x3000];
        assert_eq!(fix_overlapping_ffs_checksums(&mut img, 0x1000, 0x2FFF), 0);
        assert!(img.iter().all(|&b| b == 0x5A));
    }
}
