//! Command-line parsing, validation and help text.

use std::path::PathBuf;

/// Default header size to strip from the FRONT of the extracted .cap file before
/// the DMI transplant: 792 bytes (0x318). This is the EFI_CAPSULE_HEADER (80B) +
/// outer wrapping Firmware Volume header (72B) + FFS file header/signature blob
/// (640B) that Lenovo's InnoSetup/InsydeFlash installer prepends ahead of the real
/// flash-ready firmware image. See the `image` module docs for full verification.
pub const DEFAULT_HEADER_SIZE: usize = 0x318; // 792 bytes

/// Default target flash chip capacity for the Lenovo ADA-series (82C7/E8CN)
/// family: 8 MiB (W25Q64-class chip), measured AFTER stripping DEFAULT_HEADER_SIZE.
/// See the `image` module docs for the verification behind this number.
/// Override with --chip-size for other models.
pub const DEFAULT_CHIP_SIZE: usize = 0x0080_0000; // 8,388,608 bytes

/// Default DMI range for the ADA-series (82C7/E8CN), inclusive, relative to the
/// real (header-stripped) firmware image.
pub const DEFAULT_DMI_START: usize = 0x1000;
pub const DEFAULT_DMI_END: usize = 0x2FFF;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub old: PathBuf,
    pub new: PathBuf,
    pub out: PathBuf,
    pub start: usize,
    pub end: usize, // inclusive, matches the community guide's inclusive range convention
    pub force: bool,
    pub dry_run: bool,
    pub quiet: bool,
    pub verify_uefi: bool,
    pub fix_uefi_checksums: bool,
    pub chip_size: Option<usize>, // Some(size) = trim/pad to this size; None = --no-trim, leave as-is
    pub header_size: usize, // bytes to strip from the FRONT of --new before anything else (default 0x318)
    pub old_header_size: usize, // bytes to strip from the FRONT of --old (defaults to header_size unless overridden)
    pub show_dmi: bool, // decrypt + human-print the LENV/DMI blocks (Lenovo XOR scheme) instead of raw hex
}

/// What the command line asked for. `-h/--help` is reported as a value rather
/// than exiting from inside the parser, so parsing stays pure and testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run(Args),
    Help,
}

pub fn parse_num(s: &str) -> Result<usize, String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).map_err(|e| format!("invalid hex '{s}': {e}"))
    } else {
        s.parse::<usize>()
            .map_err(|e| format!("invalid number '{s}': {e}"))
    }
}

pub fn usage_text() -> &'static str {
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
    --start <HEX|DEC>   Start offset of DMI region, RELATIVE TO THE REAL FIRMWARE (i.e.
                        after header-stripping) (default: 0x1000, ADA-series/82C7 default)
    --end <HEX|DEC>     End offset of DMI region, inclusive, same relative basis
                        (default: 0x2FFF, ADA-series/82C7 default)
    --force             Allow overwriting --out if it already exists
    --dry-run           Show what would happen, compute diff stats, but do not write --out
    --quiet             Suppress the hex diff preview
    --verify-uefi       Scan --old and --new for UEFI Firmware Volumes / FFS files, report
                        checksum validity (real PI-spec parsing, not a guess)
    --fix-uefi-checksums
                        After patching, find any FFS file whose data region overlaps the
                        DMI range and recompute its FFS data checksum (only when the file's
                        FFS_ATTRIB_CHECKSUM bit requires one). Implies --verify-uefi.
    --header-size <HEX|DEC>
                        Bytes to strip from the FRONT of --new before anything else — this is
                        the "clean BIOS" step (default: 0x318 / 792 bytes, verified for
                        ADA-series/82C7/E8CN — see NOTES). --start/--end and UEFI scanning
                        happen after this strip, on the real firmware image.
    --old-header-size <HEX|DEC>
                        Bytes to strip from the FRONT of --old (default: same as --header-size).
                        Set this to 0 if --old is already a clean raw chip dump beginning with
                        LDBG while --new still needs the installer wrapper removed.
    --chip-size <HEX|DEC>
                        Target flash chip size in bytes — --old/--new are trimmed to this
                        length AFTER --header-size is stripped (default: 0x800000 / 8 MiB,
                        verified for ADA-series/82C7/E8CN chip family — see NOTES). Only
                        removes trailing installer/debug metadata, never touches firmware.
                        Set this if your model's chip is a different size (e.g. 16 MiB
                        chips: --chip-size 0x1000000).
    --no-trim           Disable BOTH header-stripping and chip-size trimming; use
                        --old/--new exactly as loaded/extracted. Use this if you already
                        pre-processed the files yourself, or your model's wrapper differs
                        from the verified ADA-series/82C7 layout.
    --show-dmi          Decrypt and print Lenovo LENV DMI fields in --old/--new and, after a
                        real transplant, verify the patched output against --old with explicit
                        MATCH/MISMATCH verdicts for both redundant DMI blocks.
    -h, --help          Show this help

EXAMPLE (ADA-series / 82C7 defaults: strip 792-byte header, then 8 MiB chip):
    copydmi --old dump_corrupt.bin --new e8cn39ww_extracted.bin --out fixed_bios.bin
    copydmi --old dump_corrupt.bin --new e8cn39ww.exe --out fixed_bios.bin   (feed the installer .exe directly)
    copydmi --old dump.bin --new bios.exe --out out.bin --header-size 0 --chip-size 0x1000000   (different model)
    copydmi --old dump.bin --new bios.exe --out out.bin --no-trim   (skip header-strip/trim entirely)

NOTES:
    - Offsets 0x1000-0x2FFF are specific to Lenovo ADA-series (V14/V15-ADA, IdeaPad 3 xxADA05, E8CN BIOS family),
      and are RELATIVE TO THE REAL FIRMWARE IMAGE (i.e. after the 792-byte header strip below), not the raw
      extracted .cap file. Other Lenovo models / other brands use DIFFERENT offsets and header sizes
      (e.g. some threads report DMI at 0x520000-0x5207FF with no such wrapper header at all).
      Verify your model's offset from the relevant badcaps.net / community thread before trusting the default.
    - This copies bytes [start, end] from --old into --new at the SAME offset range, then writes as --out.
    - CLEAN-HEADER STRIPPING + CHIP-SIZE TRIMMING (both default ON for ADA-series/82C7/E8CN): the .cap
      firmware image extracted from Lenovo's official installer (e.g. e8cn39ww.exe, 8,950,768 bytes) is
      NOT flash-ready as-is. It's wrapped in a 792-byte header (EFI_CAPSULE_HEADER + outer FV header + FFS
      wrapper), and the real firmware is exactly 8 MiB starting right after that header. Verified
      byte-for-byte against two real BIOS versions (e8cn39ww.exe and e8cn41ww.exe, identical header offset
      in both): every _FVH-validated Firmware Volume inside the post-strip 8 MiB slice lands on a round
      address (0x310000, 0x360000, 0x390000, 0x3A0000, 0x730000) forming a perfectly contiguous chain that
      ends EXACTLY at 0x800000, and the SMBIOS/DMI vendor string "LENV\\0" sits at relative offset 0x1000
      and 0x2000 — exactly the start and middle of the community-documented DMI range. copydmi strips
      --header-size (default 792 bytes) from the front, then trims to --chip-size (default 8 MiB) from
      the back, BEFORE doing the DMI transplant — so --out is a ready-to-flash raw image out of the box.
      Use --no-trim to skip both steps if they don't apply to your model.
"#
}

pub fn print_usage() {
    eprintln!("{}", usage_text());
}

/// Parse command-line arguments (excluding argv[0]). Arguments are processed
/// left to right: the first `-h/--help` or the first error wins.
pub fn parse_args<I>(argv: I) -> Result<Command, String>
where
    I: IntoIterator<Item = String>,
{
    let mut old: Option<PathBuf> = None;
    let mut new: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut start: usize = DEFAULT_DMI_START;
    let mut end: usize = DEFAULT_DMI_END;
    let mut force = false;
    let mut dry_run = false;
    let mut quiet = false;
    let mut verify_uefi = false;
    let mut fix_uefi_checksums = false;
    let mut chip_size: Option<usize> = Some(DEFAULT_CHIP_SIZE);
    let mut header_size: usize = DEFAULT_HEADER_SIZE;
    let mut old_header_size: Option<usize> = None; // None = use header_size (same as --new)
    let mut no_trim = false;
    let mut show_dmi = false;

    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
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
            "--header-size" => {
                header_size = parse_num(&it.next().ok_or("--header-size needs a value")?)?
            }
            "--old-header-size" => {
                old_header_size = Some(parse_num(
                    &it.next().ok_or("--old-header-size needs a value")?,
                )?)
            }
            "--chip-size" => {
                chip_size = Some(parse_num(&it.next().ok_or("--chip-size needs a value")?)?)
            }
            "--no-trim" => no_trim = true,
            "--show-dmi" => show_dmi = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    if no_trim {
        chip_size = None;
        header_size = 0;
        old_header_size = Some(0);
    }

    let old = old.ok_or("--old is required")?;
    let new = new.ok_or("--new is required")?;
    let out = out.ok_or("--out is required")?;

    validate_range(start, end, chip_size)?;

    Ok(Command::Run(Args {
        old,
        new,
        out,
        start,
        end,
        force,
        dry_run,
        quiet,
        verify_uefi,
        fix_uefi_checksums,
        chip_size,
        header_size,
        old_header_size: old_header_size.unwrap_or(header_size),
        show_dmi,
    }))
}

/// Reject DMI ranges that are inverted, or that a --chip-size trim would cut into.
pub fn validate_range(start: usize, end: usize, chip_size: Option<usize>) -> Result<(), String> {
    if start > end {
        return Err(format!(
            "--start (0x{start:X}) must be <= --end (0x{end:X})"
        ));
    }

    if let Some(size) = chip_size {
        if size <= end {
            return Err(format!(
                "--chip-size (0x{size:X}) must be greater than --end (0x{end:X}) — trimming to this \
                 size would cut off part of the DMI region itself. Use a larger --chip-size, adjust \
                 --start/--end, or pass --no-trim."
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    fn parse_run(args: &[&str]) -> Args {
        match parse(args) {
            Ok(Command::Run(a)) => a,
            other => panic!("expected Command::Run, got {other:?}"),
        }
    }

    const REQUIRED: [&str; 6] = ["--old", "o.bin", "--new", "n.bin", "--out", "p.bin"];

    fn with_required<'a>(extra: &[&'a str]) -> Vec<&'a str> {
        let mut v: Vec<&str> = REQUIRED.to_vec();
        v.extend_from_slice(extra);
        v
    }

    #[test]
    fn parse_num_accepts_hex_and_decimal() {
        assert_eq!(parse_num("0x318"), Ok(0x318));
        assert_eq!(parse_num("0X2FFF"), Ok(0x2FFF));
        assert_eq!(parse_num("0xabc"), Ok(0xABC));
        assert_eq!(parse_num("792"), Ok(792));
        assert_eq!(parse_num("  0x10  "), Ok(0x10));
        assert_eq!(parse_num("0"), Ok(0));
    }

    #[test]
    fn parse_num_rejects_garbage() {
        assert!(parse_num("zz")
            .unwrap_err()
            .starts_with("invalid number 'zz'"));
        assert!(parse_num("0xZZ")
            .unwrap_err()
            .starts_with("invalid hex '0xZZ'"));
        assert!(parse_num("0x").unwrap_err().starts_with("invalid hex '0x'"));
        assert!(parse_num("-1").is_err());
        assert!(parse_num("").is_err());
    }

    #[test]
    fn defaults_apply_when_only_required_args_given() {
        let a = parse_run(&REQUIRED);
        assert_eq!(a.old, PathBuf::from("o.bin"));
        assert_eq!(a.new, PathBuf::from("n.bin"));
        assert_eq!(a.out, PathBuf::from("p.bin"));
        assert_eq!(a.start, 0x1000);
        assert_eq!(a.end, 0x2FFF);
        assert_eq!(a.header_size, 0x318);
        assert_eq!(a.old_header_size, 0x318);
        assert_eq!(a.chip_size, Some(0x80_0000));
        assert!(!a.force && !a.dry_run && !a.quiet);
        assert!(!a.verify_uefi && !a.fix_uefi_checksums && !a.show_dmi);
    }

    #[test]
    fn required_args_are_enforced_in_order() {
        assert_eq!(parse(&[]), Err("--old is required".to_string()));
        assert_eq!(parse(&["--old", "o"]), Err("--new is required".to_string()));
        assert_eq!(
            parse(&["--old", "o", "--new", "n"]),
            Err("--out is required".to_string())
        );
    }

    #[test]
    fn missing_option_value_is_an_error() {
        for flag in [
            "--old",
            "--new",
            "--out",
            "--start",
            "--end",
            "--header-size",
            "--old-header-size",
            "--chip-size",
        ] {
            assert_eq!(parse(&[flag]), Err(format!("{flag} needs a value")));
        }
    }

    #[test]
    fn unknown_argument_is_rejected() {
        assert_eq!(
            parse(&with_required(&["--bogus"])),
            Err("unknown argument: --bogus".to_string())
        );
    }

    #[test]
    fn usage_documents_every_extended_dmi_flag() {
        let usage = usage_text();
        assert!(usage.contains("--old-header-size"));
        assert!(usage.contains("--show-dmi"));
    }

    #[test]
    fn help_wins_only_if_seen_before_an_error() {
        assert_eq!(parse(&["-h"]), Ok(Command::Help));
        assert_eq!(parse(&["--help", "--bogus"]), Ok(Command::Help));
        assert_eq!(
            parse(&["--bogus", "--help"]),
            Err("unknown argument: --bogus".to_string())
        );
    }

    #[test]
    fn fix_uefi_checksums_implies_verify() {
        let a = parse_run(&with_required(&["--fix-uefi-checksums"]));
        assert!(a.fix_uefi_checksums);
        assert!(a.verify_uefi);
    }

    #[test]
    fn simple_flags_are_set() {
        let a = parse_run(&with_required(&[
            "--force",
            "--dry-run",
            "--quiet",
            "--verify-uefi",
            "--show-dmi",
        ]));
        assert!(a.force && a.dry_run && a.quiet && a.verify_uefi && a.show_dmi);
        assert!(!a.fix_uefi_checksums);
    }

    #[test]
    fn old_header_size_follows_header_size_unless_overridden() {
        let a = parse_run(&with_required(&["--header-size", "0x100"]));
        assert_eq!((a.header_size, a.old_header_size), (0x100, 0x100));

        let a = parse_run(&with_required(&["--old-header-size", "0"]));
        assert_eq!((a.header_size, a.old_header_size), (0x318, 0));

        // Order of the two flags must not matter.
        let a = parse_run(&with_required(&[
            "--old-header-size",
            "0x20",
            "--header-size",
            "0x40",
        ]));
        assert_eq!((a.header_size, a.old_header_size), (0x40, 0x20));
    }

    #[test]
    fn no_trim_disables_strip_and_trim_regardless_of_position() {
        for extra in [
            &[
                "--no-trim",
                "--header-size",
                "5",
                "--old-header-size",
                "7",
                "--chip-size",
                "0x900000",
            ][..],
            &[
                "--header-size",
                "5",
                "--old-header-size",
                "7",
                "--chip-size",
                "0x900000",
                "--no-trim",
            ][..],
        ] {
            let a = parse_run(&with_required(extra));
            assert_eq!(a.header_size, 0);
            assert_eq!(a.old_header_size, 0);
            assert_eq!(a.chip_size, None);
        }
    }

    #[test]
    fn no_trim_skips_chip_size_validation() {
        let a = parse_run(&with_required(&["--chip-size", "0x10", "--no-trim"]));
        assert_eq!(a.chip_size, None);
    }

    #[test]
    fn custom_range_is_parsed() {
        let a = parse_run(&with_required(&["--start", "0x520000", "--end", "5376000"]));
        assert_eq!((a.start, a.end), (0x520000, 5_376_000));
    }

    #[test]
    fn bad_number_propagates_parse_error() {
        let err = parse(&with_required(&["--start", "zz"])).unwrap_err();
        assert!(err.starts_with("invalid number 'zz'"), "{err}");
    }

    #[test]
    fn validate_range_rejects_inverted_range() {
        assert_eq!(
            validate_range(0x3000, 0x2FFF, None),
            Err("--start (0x3000) must be <= --end (0x2FFF)".to_string())
        );
        assert_eq!(validate_range(0x1000, 0x1000, None), Ok(()));
    }

    #[test]
    fn validate_range_requires_chip_size_past_end() {
        let err = validate_range(0x1000, 0x2FFF, Some(0x2FFF)).unwrap_err();
        assert!(err.starts_with("--chip-size (0x2FFF) must be greater than --end (0x2FFF)"));
        assert!(validate_range(0x1000, 0x2FFF, Some(0x2FFF + 1)).is_ok());
        assert!(validate_range(0x1000, 0x2FFF, None).is_ok());
    }

    #[test]
    fn inverted_range_is_reported_before_chip_size() {
        let err = parse(&with_required(&[
            "--start",
            "0x10",
            "--end",
            "0x5",
            "--chip-size",
            "1",
        ]))
        .unwrap_err();
        assert!(err.starts_with("--start (0x10)"), "{err}");
    }
}
