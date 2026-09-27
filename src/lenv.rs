//! Lenovo LENV/DMI block decoder (XOR scheme, reverse-engineered by the
//! community — see https://github.com/Shmurkio/LenovoDMIDecryptor).
//!
//! Layout (fixed offsets, relative to the real/header-stripped firmware image):
//!   LENV Block 1 @ 0x1000, size 0x1000
//!   LENV Block 2 @ 0x2000, size 0x1000
//! Header (16 bytes, NOT encrypted):
//!   +0x00 Signature[4]   "LENV"
//!   +0x04 Generation(u32) higher = newer, 0 = invalid
//!   +0x08 Entries(u32)    total entry count
//!   +0x0C AccessFlag(u8)  bit0 = write-protected
//!   +0x0D XorKey(u8)      every body byte (everything after the header) is XORed with this
//!   +0x0E Checksum(u16)   additive 16-bit checksum of the ENCRYPTED body
//! Entry (after XOR-decrypting the body):
//!   +0x00 NamespaceId[14]
//!   +0x0E Type(u16)
//!   +0x10 DataSize(u32)
//!   +0x14 Flags(u8), Unknown1(u8), Unknown2(u16)
//!   +0x18 Data[DataSize]

pub const LENV_BLOCK1_OFFSET: usize = 0x1000;
pub const LENV_BLOCK2_OFFSET: usize = 0x2000;
const LENV_BLOCK_SIZE: usize = 0x1000;
const LENV_HEADER_LEN: usize = 16;
const LENV_ENTRY_HEADER_LEN: usize = 24;
const LENV_MAX_ENTRIES: u32 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LenvEntry {
    #[allow(dead_code)] // decoded for completeness of the entry layout; not printed or compared
    pub namespace_id: [u8; 14],
    pub entry_type: u16,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LenvBlock {
    pub generation: u32,
    pub xor_key: u8,
    pub checksum: u16,
    pub entries: Vec<LenvEntry>,
}

/// Parse one 0x1000-byte LENV block at `offset` in `buf`. Returns None if the
/// signature doesn't match (e.g. this model's DMI isn't at this offset, or
/// the block is genuinely blank/uninitialized).
pub fn parse_lenv_block(buf: &[u8], offset: usize) -> Option<LenvBlock> {
    if offset.checked_add(LENV_BLOCK_SIZE)? > buf.len() {
        return None;
    }
    let block = &buf[offset..offset + LENV_BLOCK_SIZE];
    if &block[0..4] != b"LENV" {
        return None;
    }
    let generation = u32::from_le_bytes(block[4..8].try_into().unwrap());
    let entry_count = u32::from_le_bytes(block[8..12].try_into().unwrap());
    let xor_key = block[13];
    let checksum = u16::from_le_bytes(block[14..16].try_into().unwrap());

    let body: Vec<u8> = block[LENV_HEADER_LEN..]
        .iter()
        .map(|b| b ^ xor_key)
        .collect();
    // Sanity: entry_count must be plausible, or this isn't a real LENV block.
    if entry_count > LENV_MAX_ENTRIES {
        return Some(LenvBlock {
            generation,
            xor_key,
            checksum,
            entries: Vec::new(),
        });
    }

    let mut entries = Vec::new();
    let mut pos = 0usize;
    for _ in 0..entry_count {
        if pos + LENV_ENTRY_HEADER_LEN > body.len() {
            break;
        }
        let mut namespace_id = [0u8; 14];
        namespace_id.copy_from_slice(&body[pos..pos + 14]);
        let entry_type = u16::from_le_bytes(body[pos + 14..pos + 16].try_into().unwrap());
        let data_size = u32::from_le_bytes(body[pos + 16..pos + 20].try_into().unwrap()) as usize;
        let data_start = pos + LENV_ENTRY_HEADER_LEN;
        if data_size == 0 || data_start + data_size > body.len() {
            break;
        }
        let data = body[data_start..data_start + data_size].to_vec();
        entries.push(LenvEntry {
            namespace_id,
            entry_type,
            data,
        });
        pos = data_start + data_size;
    }
    Some(LenvBlock {
        generation,
        xor_key,
        checksum,
        entries,
    })
}

/// Best-effort human label for known entry types (reverse-engineered from
/// observed 82C7/E8CN dumps — NOT an exhaustive/official Lenovo spec).
pub fn lenv_type_label(t: u16) -> &'static str {
    match t {
        0x0000 => "Product Name",
        0x0005 => "Flags/Config (raw)",
        0x000b => "Serial Number (secondary/OEM slot)",
        0x0100 => "Board/BIOS ID",
        0x0200 => "Machine Type Model (MTM)",
        0x0400 => "Serial Number",
        0x0500 => "UUID (raw 16 bytes, byte order unverified)",
        0x0700 => "Region/Flag (single char)",
        0x0b00 => "Family",
        0x1000 => "Digital Product Key (DPK)",
        _ => "(unlabeled/unknown)",
    }
}

fn hex_with_len(data: &[u8]) -> String {
    format!(
        "{} (hex, {} bytes)",
        data.iter().map(|b| format!("{b:02X}")).collect::<String>(),
        data.len()
    )
}

pub fn format_lenv_data(t: u16, data: &[u8]) -> String {
    if t == 0x0500 {
        return hex_with_len(data);
    }
    // Printable ASCII heuristic
    if !data.is_empty() && data.iter().all(|&b| (0x20..0x7f).contains(&b) || b == 0) {
        let s: String = data
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        if !s.is_empty() {
            return format!("{s:?}");
        }
    }
    hex_with_len(data)
}

/// True if both blocks decode to the same entries (same type + same data, in
/// order), or if neither location holds a LENV block at all. This is what "the
/// transplant carried the real DMI through unchanged" means.
pub fn blocks_match(a: &Option<LenvBlock>, b: &Option<LenvBlock>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => {
            x.entries.len() == y.entries.len()
                && x.entries
                    .iter()
                    .zip(y.entries.iter())
                    .all(|(ea, eb)| ea.entry_type == eb.entry_type && ea.data == eb.data)
        }
        (None, None) => true,
        _ => false,
    }
}

fn print_lenv_block(label: &str, offset: usize, block: &Option<LenvBlock>) {
    match block {
        None => println!("  [{label} @0x{offset:04X}] not a LENV block (no 'LENV' signature, or DMI isn't at this offset for this model)"),
        Some(b) => {
            println!(
                "  [{label} @0x{offset:04X}] generation={} xor_key=0x{:02X} checksum=0x{:04X} entries={}",
                b.generation, b.xor_key, b.checksum, b.entries.len()
            );
            for e in &b.entries {
                println!(
                    "      type=0x{:04X} [{:<38}] = {}",
                    e.entry_type,
                    lenv_type_label(e.entry_type),
                    format_lenv_data(e.entry_type, &e.data)
                );
            }
        }
    }
}

fn print_verdict_line(offset: usize, ok: bool) {
    println!(
        "  block @0x{offset:04X}: {}",
        if ok {
            "MATCH — patched output carries the real old-bios DMI values unchanged"
        } else {
            "MISMATCH — patched output DOES NOT match old-bios DMI, do not flash"
        }
    );
}

/// Show + compare decrypted DMI (LENV) blocks for old/new/patched, so the
/// operator sees human-readable Serial/MTM/Product/UUID values (not just raw
/// hex bytes) and an explicit PASS/FAIL on whether the transplant carried the
/// real values through unchanged.
pub fn dmi_report(old_buf: &[u8], new_buf: &[u8], patched: Option<&[u8]>) {
    println!("=== DMI (LENV) decode — Lenovo XOR scheme ===");
    let old_b1 = parse_lenv_block(old_buf, LENV_BLOCK1_OFFSET);
    let old_b2 = parse_lenv_block(old_buf, LENV_BLOCK2_OFFSET);
    println!("-- old bios --");
    print_lenv_block("old", LENV_BLOCK1_OFFSET, &old_b1);
    print_lenv_block("old", LENV_BLOCK2_OFFSET, &old_b2);

    println!("-- new bios (pre-patch) --");
    let new_b1 = parse_lenv_block(new_buf, LENV_BLOCK1_OFFSET);
    let new_b2 = parse_lenv_block(new_buf, LENV_BLOCK2_OFFSET);
    print_lenv_block("new", LENV_BLOCK1_OFFSET, &new_b1);
    print_lenv_block("new", LENV_BLOCK2_OFFSET, &new_b2);

    if let Some(p) = patched {
        println!("-- patched output --");
        let p_b1 = parse_lenv_block(p, LENV_BLOCK1_OFFSET);
        let p_b2 = parse_lenv_block(p, LENV_BLOCK2_OFFSET);
        print_lenv_block("patched", LENV_BLOCK1_OFFSET, &p_b1);
        print_lenv_block("patched", LENV_BLOCK2_OFFSET, &p_b2);

        // Verdict: for each block, do the patched entries match old's entries
        // byte-for-byte (same data).
        let ok1 = blocks_match(&old_b1, &p_b1);
        let ok2 = blocks_match(&old_b2, &p_b2);
        println!("-- verdict --");
        print_verdict_line(LENV_BLOCK1_OFFSET, ok1);
        print_verdict_line(LENV_BLOCK2_OFFSET, ok2);
        if !ok1 || !ok2 {
            println!("  ################ WARNING: DMI VERIFICATION FAILED — DO NOT FLASH THIS OUTPUT ################");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an encrypted 0x1000-byte LENV block from (type, data) entries.
    fn build_block(generation: u32, xor_key: u8, entries: &[(u16, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (t, data) in entries {
            let mut ns = [0u8; 14];
            ns[..3].copy_from_slice(b"NS1");
            body.extend_from_slice(&ns);
            body.extend_from_slice(&t.to_le_bytes());
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(&[0u8; 4]);
            body.extend_from_slice(data);
        }
        body.resize(LENV_BLOCK_SIZE - LENV_HEADER_LEN, 0xFF);
        let enc: Vec<u8> = body.iter().map(|b| b ^ xor_key).collect();
        let checksum = enc.iter().fold(0u16, |s, &b| s.wrapping_add(b as u16));

        let mut block = b"LENV".to_vec();
        block.extend_from_slice(&generation.to_le_bytes());
        block.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        block.push(0); // access flag
        block.push(xor_key);
        block.extend_from_slice(&checksum.to_le_bytes());
        block.extend_from_slice(&enc);
        assert_eq!(block.len(), LENV_BLOCK_SIZE);
        block
    }

    fn image_with_block(block: &[u8]) -> Vec<u8> {
        let mut img = vec![0xFFu8; 0x3000];
        img[LENV_BLOCK1_OFFSET..LENV_BLOCK1_OFFSET + LENV_BLOCK_SIZE].copy_from_slice(block);
        img
    }

    #[test]
    fn decodes_xor_encrypted_entries() {
        let block = build_block(7, 0x5A, &[(0x0400, b"PF2ABCDE\0"), (0x0500, &[0xAB; 16])]);
        let img = image_with_block(&block);
        let b = parse_lenv_block(&img, LENV_BLOCK1_OFFSET).expect("LENV block");
        assert_eq!(b.generation, 7);
        assert_eq!(b.xor_key, 0x5A);
        assert_eq!(b.checksum, u16::from_le_bytes([block[14], block[15]]));
        assert_eq!(b.entries.len(), 2);
        assert_eq!(b.entries[0].entry_type, 0x0400);
        assert_eq!(b.entries[0].data, b"PF2ABCDE\0");
        assert_eq!(&b.entries[0].namespace_id[..3], b"NS1");
        assert_eq!(b.entries[1].entry_type, 0x0500);
        assert_eq!(b.entries[1].data, vec![0xAB; 16]);
    }

    #[test]
    fn rejects_missing_signature() {
        let img = vec![0u8; 0x3000];
        assert_eq!(parse_lenv_block(&img, LENV_BLOCK1_OFFSET), None);
    }

    #[test]
    fn rejects_block_past_end_of_buffer() {
        let block = build_block(1, 0x11, &[(0, b"x")]);
        let img = image_with_block(&block);
        assert_eq!(parse_lenv_block(&img, 0x2001), None);
        assert_eq!(parse_lenv_block(&img[..0x1FFF], LENV_BLOCK1_OFFSET), None);
        assert_eq!(parse_lenv_block(&img, usize::MAX), None);
    }

    #[test]
    fn implausible_entry_count_yields_empty_block() {
        let mut block = build_block(3, 0x22, &[(0, b"abc")]);
        block[8..12].copy_from_slice(&257u32.to_le_bytes());
        let img = image_with_block(&block);
        let b = parse_lenv_block(&img, LENV_BLOCK1_OFFSET).unwrap();
        assert_eq!(b.generation, 3);
        assert!(b.entries.is_empty());
    }

    #[test]
    fn stops_at_zero_sized_or_overflowing_entry() {
        // Header claims 3 entries, but the 2nd has DataSize 0: only the 1st is kept.
        let mut block = build_block(1, 0x00, &[(0x0000, b"Name\0"), (0x0400, b"")]);
        block[8..12].copy_from_slice(&3u32.to_le_bytes());
        let b = parse_lenv_block(&image_with_block(&block), LENV_BLOCK1_OFFSET).unwrap();
        assert_eq!(b.entries.len(), 1);

        // DataSize larger than the remaining body: nothing is decoded.
        let mut block = build_block(1, 0x00, &[(0x0000, b"x")]);
        let huge = (LENV_BLOCK_SIZE as u32).to_le_bytes();
        block[LENV_HEADER_LEN + 16..LENV_HEADER_LEN + 20].copy_from_slice(&huge);
        let b = parse_lenv_block(&image_with_block(&block), LENV_BLOCK1_OFFSET).unwrap();
        assert!(b.entries.is_empty());
    }

    #[test]
    fn type_labels() {
        assert_eq!(lenv_type_label(0x0400), "Serial Number");
        assert_eq!(lenv_type_label(0x0200), "Machine Type Model (MTM)");
        assert_eq!(lenv_type_label(0xBEEF), "(unlabeled/unknown)");
    }

    #[test]
    fn formats_ascii_uuid_and_binary_data() {
        assert_eq!(format_lenv_data(0x0400, b"PF2ABCDE\0\0"), "\"PF2ABCDE\"");
        assert_eq!(format_lenv_data(0x0500, b"ABCD"), "41424344 (hex, 4 bytes)");
        assert_eq!(
            format_lenv_data(0x1234, &[0x01, 0xFF]),
            "01FF (hex, 2 bytes)"
        );
        // All-NUL data is "printable" but yields an empty string -> hex fallback.
        assert_eq!(format_lenv_data(0x0000, &[0, 0]), "0000 (hex, 2 bytes)");
        assert_eq!(format_lenv_data(0x0000, &[]), " (hex, 0 bytes)");
    }

    fn block_of(entries: &[(u16, &[u8])]) -> Option<LenvBlock> {
        Some(LenvBlock {
            generation: 1,
            xor_key: 0,
            checksum: 0,
            entries: entries
                .iter()
                .map(|(t, d)| LenvEntry {
                    namespace_id: [0; 14],
                    entry_type: *t,
                    data: d.to_vec(),
                })
                .collect(),
        })
    }

    #[test]
    fn blocks_match_compares_type_and_data_only() {
        let a = block_of(&[(0x0400, b"SN1")]);
        let mut b = block_of(&[(0x0400, b"SN1")]);
        // Header fields and namespace are not part of the verdict.
        if let Some(blk) = b.as_mut() {
            blk.generation = 9;
            blk.entries[0].namespace_id = [1; 14];
        }
        assert!(blocks_match(&a, &b));
        assert!(!blocks_match(&a, &block_of(&[(0x0400, b"SN2")])));
        assert!(!blocks_match(&a, &block_of(&[(0x0200, b"SN1")])));
        assert!(!blocks_match(&a, &block_of(&[(0x0400, b"SN1"), (0, b"x")])));
    }

    #[test]
    fn blocks_match_handles_absent_blocks() {
        assert!(blocks_match(&None, &None));
        assert!(!blocks_match(&block_of(&[]), &None));
        assert!(!blocks_match(&None, &block_of(&[])));
    }
}
