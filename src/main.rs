// copydmi - Rust CLI to transplant DMI/SMBIOS info block between Lenovo BIOS dumps
// Use case: Lenovo V15-ADA / V14-ADA / IdeaPad 3 xxADA05 (E8CN series, chip 82C7 etc)
// Replicates the community "CopyDMI" macro workflow (Tiny Hexer) from badcaps.net,
// but as a scriptable/auditable CLI instead of a black-box .mps macro file.
//
// DEFAULT OFFSETS are for the ADA-series (82C7 / E8CN) BIOS: DMI region 0x1000-0x2FFF
// (per badcaps.net thread "IdeaPad 3 14ADA5/15ADA05/17ADA05 Lenovo V14-ADA/V15-ADA Bios").
// ALWAYS double check offsets against your own verified source before trusting blindly.
//
// USAGE:
//   copydmi --old oldbios.bin --new newbios.bin --out patched.bin
//   copydmi --old oldbios.bin --new newbios.bin --out patched.bin --start 0x1000 --end 0x2FFF
//   copydmi --old oldbios.bin --new newbios.bin --out patched.bin --dry-run
//
// SAFETY:
//   - Never overwrites newbios.bin/oldbios.bin in place; always writes to --out.
//   - Refuses to run if --out already exists, unless --force is passed.
//   - Verifies newbios.bin size before and after patch matches (BIOS image size must not change).
//   - Prints a hex diff summary of the patched region before writing, unless --quiet.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

struct Args {
    old: PathBuf,
    new: PathBuf,
    out: PathBuf,
    start: usize,
    end: usize, // inclusive, matches the community guide's inclusive range convention
    force: bool,
    dry_run: bool,
    quiet: bool,
    verify_uefi: bool,
    fix_uefi_checksums: bool,
}

// ============================================================================
// UEFI Firmware Volume (FV) + Firmware File System (FFS) checksum engine
// ============================================================================
// This implements REAL structural parsing per the UEFI PI spec (Volume 3):
//   - EFI_FIRMWARE_VOLUME_HEADER: locates FVs by scanning for the "_FVH"
//     signature, validates the 16-bit header checksum (sum of all UINT16
//     words across HeaderLength must equal 0 mod 0x10000).
//   - EFI_FFS_FILE_HEADER (non-extended, 24-byte, Size < 0xFFFFFF): walks
//     files sequentially inside a FV's body (8-byte aligned), validates:
//       * Header checksum (IntegrityCheck.Header): sum of all header bytes,
//         with State and IntegrityCheck.File treated as 0 during the sum,
//         must be 0 mod 256.
//       * Data checksum (IntegrityCheck.File): only meaningful if
//         FFS_ATTRIB_CHECKSUM (0x40) is set in Attributes; otherwise the
//         spec fixes it at 0xAA (not a real checksum). When set, sum of all
//         data bytes must be 0 mod 256.
//
// This is why the community's raw byte-swap trick to move DMI often "just
// works": on these Lenovo AMD ADA-series (Insyde H2O) images the DMI data
// commonly sits in an FFS file WITHOUT FFS_ATTRIB_CHECKSUM set, so no data
// checksum is enforced there at all. But we don't assume that — we actually
// parse the structures and tell you the truth for YOUR specific files.
//
// Scope/limits (stated honestly, not hidden):
//   - Only classic (non-extended) FFS file headers are handled (24-byte,
//     Size field < 0xFFFFFF). Large files using the extended 32-byte header
//     are detected and reported as "skipped (extended header, unsupported)".
//   - We do not parse FFS section internals (PE32/raw/etc) or recompute any
//     GUIDed-section (e.g. Tiano-compressed) internal checksums.
//   - We do not touch Volume Top File / other vendor-specific signing.
//   - This does NOT replace UEFITool for deep firmware surgery — it targets
//     exactly the case this tool exists for: verifying/fixing the FFS file
//     whose data region overlaps the DMI byte range we just patched.

#[derive(Debug, Clone)]
struct FvInfo {
    fv_start: usize,
    fv_length: usize,
    header_length: usize,
    header_checksum_ok: bool,
}

#[derive(Debug, Clone)]
struct FfsInfo {
    file_start: usize,   // offset of EFI_FFS_FILE_HEADER
    header_len: usize,   // 24 (classic) — extended not supported
    data_start: usize,
    data_len: usize,
    header_checksum_ok: bool,
    checksum_attrib_set: bool,
    data_checksum_ok: Option<bool>, // None if attrib not set (0xAA fixed, not a real checksum)
}

const FV_SIG: &[u8; 4] = b"_FVH";

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
fn scan_fvs(buf: &[u8]) -> Vec<FvInfo> {
    let mut out = Vec::new();
    if buf.len() < 64 {
        return out;
    }
    let mut i = 0usize;
    while i + 4 <= buf.len().saturating_sub(40) {
        if &buf[i + 40..i + 44] == FV_SIG {
            // candidate FV start = i
            if i + 56 <= buf.len() {
                let fv_length = u64::from_le_bytes(buf[i + 32..i + 40].try_into().unwrap()) as usize;
                let header_length = u16::from_le_bytes(buf[i + 48..i + 50].try_into().unwrap()) as usize;
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
                    let hdr = &buf[i..i + header_length];
                    let ok = checksum16_zero(hdr);
                    out.push(FvInfo {
                        fv_start: i,
                        fv_length,
                        header_length,
                        header_checksum_ok: ok,
                    });
                }
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
fn checksum16_zero(data: &[u8]) -> bool {
    let mut sum: u32 = 0;
    let mut chunks = data.chunks_exact(2);
    for c in &mut chunks {
        sum = sum.wrapping_add(u16::from_le_bytes([c[0], c[1]]) as u32);
    }
    if let [last] = chunks.remainder() {
        sum = sum.wrapping_add(*last as u32);
    }
    (sum & 0xFFFF) == 0
}

fn sum8_zero(data: &[u8]) -> bool {
    let mut sum: u8 = 0;
    for &b in data {
        sum = sum.wrapping_add(b);
    }
    sum == 0
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
fn scan_ffs_in_fv(buf: &[u8], fv: &FvInfo) -> Vec<FfsInfo> {
    let mut out = Vec::new();
    let body_start = fv.fv_start + fv.header_length;
    let body_end = fv.fv_start + fv.fv_length;
    if body_end > buf.len() || body_start >= body_end {
        return out;
    }
    let mut pos = body_start;
    // align up to 8 bytes from FV body start, per spec files are 8-byte aligned
    if pos % 8 != 0 {
        pos += 8 - (pos % 8);
    }
    const FFS_HDR_LEN: usize = 24;
    const ERASE_POLARITY_FF_RUN_CHECK: usize = 16; // how many bytes of 0xFF/0x00 before declaring "end of files"
    while pos + FFS_HDR_LEN <= body_end {
        let hdr = &buf[pos..pos + FFS_HDR_LEN];
        // detect padding/empty space: run of 0xFF (erased flash) or all 0x00
        let all_ff = hdr.iter().all(|&b| b == 0xFF);
        let all_00 = hdr.iter().all(|&b| b == 0x00);
        if all_ff || all_00 {
            // check a bit further to decide if this is real trailing padding
            let probe_end = (pos + ERASE_POLARITY_FF_RUN_CHECK).min(body_end);
            let probe = &buf[pos..probe_end];
            let probe_uniform = probe.iter().all(|&b| b == hdr[0]);
            if probe_uniform {
                break; // reached padding/free space, stop walking this FV
            }
        }

        let size_bytes = [hdr[0x14], hdr[0x15], hdr[0x16], 0u8];
        let size = u32::from_le_bytes(size_bytes) as usize;
        let attributes = hdr[0x13];
        let large_file = size == 0x00FF_FFFF || size == 0; // 0xFFFFFF marker or garbage -> extended/unsupported
        if large_file || size < FFS_HDR_LEN || pos + size > body_end {
            // can't safely continue walking this FV; stop (extended header or corrupt/unknown)
            break;
        }

        // Header checksum: sum of header bytes with State and IntegrityCheck.File zeroed
        let mut hdr_for_check = hdr.to_vec();
        hdr_for_check[0x11] = 0; // IntegrityCheck.File
        hdr_for_check[0x17] = 0; // State
        let header_checksum_ok = sum8_zero(&hdr_for_check);

        let checksum_attrib_set = attributes & 0x40 != 0; // FFS_ATTRIB_CHECKSUM
        let data_start = pos + FFS_HDR_LEN;
        let data_len = size - FFS_HDR_LEN;
        let data_checksum_ok = if checksum_attrib_set && data_start + data_len <= buf.len() {
            Some(sum8_zero(&buf[data_start..data_start + data_len]))
        } else {
            None
        };

        out.push(FfsInfo {
            file_start: pos,
            header_len: FFS_HDR_LEN,
            data_start,
            data_len,
            header_checksum_ok,
            checksum_attrib_set,
            data_checksum_ok,
        });

        pos += size;
        if pos % 8 != 0 {
            pos += 8 - (pos % 8);
        }
    }
    out
}

fn uefi_report(label: &str, buf: &[u8]) -> Vec<(FvInfo, Vec<FfsInfo>)> {
    let fvs = scan_fvs(buf);
    println!("  -- UEFI structure scan: {label} ({} bytes) --", buf.len());
    if fvs.is_empty() {
        println!("    no Firmware Volume (_FVH) headers found (raw/non-UEFI image, or none detected).");
        return Vec::new();
    }
    let mut results = Vec::new();
    for fv in &fvs {
        let ffs_list = scan_ffs_in_fv(buf, fv);
        println!(
            "    FV @0x{:06X} len=0x{:X} header_len={} header_checksum={}",
            fv.fv_start,
            fv.fv_length,
            fv.header_length,
            if fv.header_checksum_ok { "OK" } else { "BAD" }
        );
        println!("      {} FFS files parsed (classic headers only)", ffs_list.len());
        let bad_headers = ffs_list.iter().filter(|f| !f.header_checksum_ok).count();
        let bad_data = ffs_list
            .iter()
            .filter(|f| f.data_checksum_ok == Some(false))
            .count();
        if bad_headers > 0 || bad_data > 0 {
            println!("      WARNING: {bad_headers} file(s) with bad header checksum, {bad_data} file(s) with bad data checksum");
        }
        results.push((fv.clone(), ffs_list));
    }
    results
}

/// After patching bytes in [patch_start, patch_end], find any FFS file(s)
/// whose data region overlaps the patched range, and recompute their
/// IntegrityCheck.File data checksum (only meaningful if FFS_ATTRIB_CHECKSUM
/// is set — otherwise per spec it must stay fixed at 0xAA, so we leave it).
/// Returns how many files were fixed.
fn fix_overlapping_ffs_checksums(
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
            let file_checksum_off = f.file_start + 0x11;
            buf[file_checksum_off] = 0;
            let mut sum: u8 = 0;
            for &b in &buf[f.data_start..f.data_start + f.data_len] {
                sum = sum.wrapping_add(b);
            }
            buf[file_checksum_off] = (0u8).wrapping_sub(sum);
            fixed += 1;
        }
    }
    fixed
}

fn parse_num(s: &str) -> Result<usize, String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).map_err(|e| format!("invalid hex '{s}': {e}"))
    } else {
        s.parse::<usize>().map_err(|e| format!("invalid number '{s}': {e}"))
    }
}

fn print_usage() {
    eprintln!(
        r#"copydmi - transplant DMI/SMBIOS block between Lenovo BIOS dumps

USAGE:
    copydmi --old <oldbios.bin> --new <newbios.bin> --out <patched.bin> [OPTIONS]

REQUIRED:
    --old <FILE>     Path to your ORIGINAL bios dump (has your real serial/DMI, may be corrupt elsewhere)
                     Accepts a raw dump (.bin/.cap/.fd/.rom) OR a Lenovo installer .exe (auto-extracted)
    --new <FILE>     Path to the CLEAN/official bios you downloaded (e.g. e8cn39ww / e8cn41ww)
                     Accepts a raw dump (.bin/.cap/.fd/.rom) OR a Lenovo installer .exe directly —
                     if it ends in .exe, the firmware image is extracted in-memory automatically
                     (pure Rust, no innoextract/7z/Docker needed)
    --out <FILE>     Output path for the patched result (never overwrites inputs)

OPTIONS:
    --start <HEX|DEC>   Start offset of DMI region (default: 0x1000, ADA-series/82C7 default)
    --end <HEX|DEC>     End offset of DMI region, inclusive (default: 0x2FFF, ADA-series/82C7 default)
    --force             Allow overwriting --out if it already exists
    --dry-run           Show what would happen, compute diff stats, but do not write --out
    --quiet             Suppress the hex diff preview
    --verify-uefi       Scan --old and --new for UEFI Firmware Volumes / FFS files, report
                        checksum validity (real PI-spec parsing, not a guess)
    --fix-uefi-checksums
                        After patching, find any FFS file whose data region overlaps the
                        DMI range and recompute its FFS data checksum (only when the file's
                        FFS_ATTRIB_CHECKSUM bit requires one). Implies --verify-uefi.
    -h, --help          Show this help

EXAMPLE (ADA-series / 82C7 default offsets):
    copydmi --old dump_corrupt.bin --new e8cn39ww_extracted.bin --out fixed_bios.bin
    copydmi --old dump_corrupt.bin --new e8cn39ww.exe --out fixed_bios.bin   (feed the installer .exe directly)

NOTES:
    - Offsets 0x1000-0x2FFF are specific to Lenovo ADA-series (V14/V15-ADA, IdeaPad 3 xxADA05, E8CN BIOS family).
      Other Lenovo models / other brands use DIFFERENT offsets (e.g. some threads report 0x520000-0x5207FF).
      Verify your model's offset from the relevant badcaps.net / community thread before trusting the default.
    - This copies bytes [start, end] from --old into --new at the SAME offset range, then writes as --out.
    - Both files must be at least (end+1) bytes long, and --new must be the same total size as a valid
      full BIOS image for your chip (do not feed it a truncated dump).
"#
    );
}

fn parse_args() -> Result<Args, String> {
    let mut old: Option<PathBuf> = None;
    let mut new: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut start: usize = 0x1000;
    let mut end: usize = 0x2FFF;
    let mut force = false;
    let mut dry_run = false;
    let mut quiet = false;
    let mut verify_uefi = false;
    let mut fix_uefi_checksums = false;

    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "--old" => old = Some(PathBuf::from(it.next().ok_or("--old needs a value")?)),
            "--new" => new = Some(PathBuf::from(it.next().ok_or("--new needs a value")?)),
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a value")?)),
            "--start" => start = parse_num(&it.next().ok_or("--start needs a value")?)?,
            "--end" => end = parse_num(&it.next().ok_or("--end needs a value")?)?,
            "--force" => force = true,
            "--dry-run" => dry_run = true,
            "--quiet" => quiet = true,
            "--verify-uefi" => verify_uefi = true,
            "--fix-uefi-checksums" => {
                fix_uefi_checksums = true;
                verify_uefi = true;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let old = old.ok_or("--old is required")?;
    let new = new.ok_or("--new is required")?;
    let out = out.ok_or("--out is required")?;

    if start > end {
        return Err(format!("--start (0x{start:X}) must be <= --end (0x{end:X})"));
    }

    Ok(Args { old, new, out, start, end, force, dry_run, quiet, verify_uefi, fix_uefi_checksums })
}

fn hex_preview(label: &str, buf: &[u8], start: usize, len_show: usize) {
    let end = (start + len_show).min(buf.len());
    print!("  {label} @0x{start:06X}: ");
    for b in &buf[start..end] {
        print!("{b:02X} ");
    }
    println!();
}

// ============================================================================
// Flexible input loading: --old / --new accept EITHER a raw BIOS image
// (.cap/.fd/.bin/.rom/anything) OR a Lenovo BIOS update installer .exe
// (Inno Setup wrapper). When given a .exe, the firmware image is extracted
// in-memory using the pure-Rust `inno` crate (same logic as the standalone
// lenovo-bios-extract-rs tool) — no external innoextract/7z/Docker needed.
// ============================================================================

const FW_EXT_TIERS: &[&[&str]] = &[&["cap"], &["fd"], &["bin", "rom"]];
const FW_MIN_PLAUSIBLE_SIZE: u64 = 1024 * 1024; // 1 MiB floor, rejects install.bat etc

fn fw_ext_tier(name: &str) -> Option<usize> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    FW_EXT_TIERS.iter().position(|tier| tier.contains(&ext.as_str()))
}

/// Extract the most plausible firmware image out of a Lenovo BIOS installer
/// .exe (Inno Setup archive), fully in-memory, no temp files written.
/// Same selection heuristic as lenovo-bios-extract-rs: prefer .cap > .fd >
/// .bin/.rom, pick the largest candidate >= 1 MiB within the best tier.
fn extract_firmware_from_exe(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path)
        .map_err(|e| format!("failed to open installer '{}': {e}", path.display()))?;
    let mut inno = inno::Inno::new(file)
        .map_err(|e| format!("failed to parse '{}' as Inno Setup installer: {e}", path.display()))?;

    println!(
        "  (detected .exe, extracting embedded firmware via pure-Rust Inno parser — Inno Setup version {})",
        inno.version()
    );

    // Pass 1: find best (tier, size) candidate name among embedded files.
    let mut best_name: Option<String> = None;
    let mut best_tier: usize = usize::MAX;
    let mut best_size: u64 = 0;
    {
        let files = inno.filtered_files(|file_entry| {
            file_entry
                .file()
                .destination()
                .is_some_and(|d| fw_ext_tier(d).is_some())
        });
        for res in files {
            let (entry, data) = res.map_err(|e| format!("error reading installer entry: {e}"))?;
            let Some(dest) = entry.file().normalized_destination() else { continue };
            if dest.is_empty() {
                continue;
            }
            let Some(tier) = fw_ext_tier(&dest) else { continue };
            let size = data.len() as u64;
            if size < FW_MIN_PLAUSIBLE_SIZE {
                continue;
            }
            let better = tier < best_tier || (tier == best_tier && size > best_size);
            if better {
                best_tier = tier;
                best_size = size;
                best_name = Some(dest);
            }
        }
    }

    let Some(target_name) = best_name else {
        return Err(format!(
            "no plausible firmware image (*.cap/*.fd/*.bin/*.rom >= {} bytes) found inside installer '{}'. \
             Try lenovo-bios-extract-rs --list on it to inspect all embedded files.",
            FW_MIN_PLAUSIBLE_SIZE,
            path.display()
        ));
    };

    // Pass 2: re-extract just that file's bytes.
    let mut result: Option<Vec<u8>> = None;
    {
        let files = inno.filtered_files(|file_entry| {
            file_entry
                .file()
                .destination()
                .is_some_and(|d| fw_ext_tier(d).is_some())
        });
        for res in files {
            let (entry, data) = res.map_err(|e| format!("error reading installer entry: {e}"))?;
            let Some(dest) = entry.file().normalized_destination() else { continue };
            if dest != target_name {
                continue;
            }
            result = Some(data);
            break;
        }
    }

    let data = result.ok_or_else(|| {
        "internal error: selected firmware entry could not be re-extracted".to_string()
    })?;

    println!(
        "  (extracted '{target_name}' from installer, {} bytes)",
        data.len()
    );

    Ok(data)
}

/// Load a BIOS image from disk. If the path ends in .exe, transparently
/// extract the embedded firmware image in-memory first (see
/// extract_firmware_from_exe). Otherwise the file is read as a raw BIOS
/// image directly (.cap/.fd/.bin/.rom/anything else).
fn load_bios_buf(path: &std::path::Path, label: &str) -> Result<Vec<u8>, String> {
    let is_exe = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("exe"))
        .unwrap_or(false);

    if is_exe {
        extract_firmware_from_exe(path)
            .map_err(|e| format!("failed to extract firmware from --{label} installer: {e}"))
    } else {
        fs::read(path).map_err(|e| format!("failed to read --{label} '{}': {e}", path.display()))
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    if args.out.exists() && !args.force {
        return Err(format!(
            "output file '{}' already exists. Use --force to overwrite, or pick a different --out path.",
            args.out.display()
        ));
    }

    let old_buf = load_bios_buf(&args.old, "old")?;
    let new_buf = load_bios_buf(&args.new, "new")?;

    let region_len = args.end - args.start + 1;

    if old_buf.len() <= args.end {
        return Err(format!(
            "old bios file is too small ({} bytes) for DMI range ending at 0x{:X}",
            old_buf.len(),
            args.end
        ));
    }
    if new_buf.len() <= args.end {
        return Err(format!(
            "new bios file is too small ({} bytes) for DMI range ending at 0x{:X}",
            new_buf.len(),
            args.end
        ));
    }

    let old_region = &old_buf[args.start..=args.end];
    let new_region = &new_buf[args.start..=args.end];

    let diff_bytes = old_region
        .iter()
        .zip(new_region.iter())
        .filter(|(a, b)| a != b)
        .count();

    println!("=== copydmi: DMI transplant ===");
    println!("  old bios : {} ({} bytes)", args.old.display(), old_buf.len());
    println!("  new bios : {} ({} bytes)", args.new.display(), new_buf.len());
    println!("  DMI range: 0x{:06X} - 0x{:06X} ({} bytes)", args.start, args.end, region_len);
    println!("  bytes differing in this range: {diff_bytes} / {region_len}");

    if new_buf.len() != old_buf.len() {
        println!(
            "  WARNING: old and new bios total sizes differ ({} vs {} bytes). \
             This is common (different BIOS versions) but double-check both are full, \
             untruncated dumps for the SAME chip capacity before flashing.",
            old_buf.len(),
            new_buf.len()
        );
    }

    if diff_bytes == 0 {
        println!("  NOTE: DMI region is already identical between old and new — nothing to transplant.");
    }

    if !args.quiet {
        println!("  -- preview (first 32 bytes of range) --");
        hex_preview("old", &old_buf, args.start, 32.min(region_len));
        hex_preview("new (before patch)", &new_buf, args.start, 32.min(region_len));
    }

    if args.verify_uefi {
        uefi_report("old bios", &old_buf);
        uefi_report("new bios (pre-patch)", &new_buf);
    }

    if args.dry_run {
        println!("  (dry-run) no output written.");
        return Ok(());
    }

    let mut patched = new_buf.clone();
    patched[args.start..=args.end].copy_from_slice(old_region);

    if patched.len() != new_buf.len() {
        return Err("internal error: patched size does not match new bios size".to_string());
    }

    if args.fix_uefi_checksums {
        let fixed = fix_overlapping_ffs_checksums(&mut patched, args.start, args.end);
        if fixed > 0 {
            println!(
                "  UEFI checksum fixup: recomputed IntegrityCheck.File data checksum for {fixed} FFS file(s) \
                 overlapping the DMI range (only files with FFS_ATTRIB_CHECKSUM set needed this)."
            );
        } else {
            println!(
                "  UEFI checksum fixup: no FFS file overlapping the DMI range required a data checksum fix \
                 (either no FV/FFS structure detected, or the overlapping file(s) don't set FFS_ATTRIB_CHECKSUM \
                 — meaning their IntegrityCheck.File byte is a fixed 0xAA per spec and isn't a real checksum)."
            );
        }
    }

    fs::write(&args.out, &patched)
        .map_err(|e| format!("failed to write --out '{}': {e}", args.out.display()))?;

    if !args.quiet {
        println!("  -- preview (first 32 bytes of range, after patch) --");
        hex_preview("out", &patched, args.start, 32.min(region_len));
    }

    if args.verify_uefi {
        uefi_report("patched output", &patched);
    }

    println!("  wrote patched bios: {} ({} bytes)", args.out.display(), patched.len());
    println!(
        "  DONE. Verify with a hex editor / diff before flashing to the chip. \
         This tool swaps the DMI byte range and (if --fix-uefi-checksums was passed) repairs \
         FFS-level data checksums for files overlapping that range. It does NOT validate FV \
         header checksums beyond reporting them, does NOT handle extended FFS headers, GUIDed/\
         compressed sections, Insyde vendor signing, or secure-boot keys."
    );

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("run with -h for usage");
            ExitCode::FAILURE
        }
    }
}
