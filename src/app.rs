//! The copydmi pipeline: load → strip header → trim to chip size → report →
//! transplant → optional UEFI checksum fixup → write → verify.

use std::fs;

use crate::cli::Args;
use crate::image::{self, Trim};
use crate::lenv::{dmi_report, require_old_dmi, verify_patched_dmi};
use crate::loader::load_bios_buf;
use crate::transplant::{count_differing, hex_preview_line, transplant};
use crate::uefi::{fix_overlapping_ffs_checksums, uefi_report};

/// Bytes of the DMI range shown in the hex previews.
const PREVIEW_LEN: usize = 32;

pub fn run(args: &Args) -> Result<(), String> {
    if args.out.exists() && !args.force {
        return Err(format!(
            "output file '{}' already exists. Use --force to overwrite, or pick a different --out path.",
            args.out.display()
        ));
    }

    let mut old_buf = load_bios_buf(&args.old, "old")?;
    let mut new_buf = load_bios_buf(&args.new, "new")?;

    // STEP 1: Strip the wrapper header from the FRONT (the "clean BIOS" step).
    // Lenovo's InnoSetup/InsydeFlash installer prepends a fixed-size header
    // (EFI_CAPSULE_HEADER + outer FV header + FFS wrapper, verified 792 bytes
    // for ADA-series/82C7/E8CN) ahead of the real flash-ready firmware image.
    // Everything downstream (--start/--end DMI range, UEFI FV/FFS scanning,
    // --chip-size trimming) operates on the buffer AFTER this strip.
    if args.header_size > 0 || args.old_header_size > 0 {
        for (label, buf, hsize) in [
            ("old", &mut old_buf, args.old_header_size),
            ("new", &mut new_buf, args.header_size),
        ] {
            if hsize == 0 {
                println!("  (--{label}: header-size is 0, skipping front-strip for this file)");
                continue;
            }
            image::strip_header(buf, hsize, label)?;
            println!(
                "  (stripping --{label}: removing {} bytes of capsule/wrapper header from the front \
                 — \"clean BIOS\" step)",
                hsize
            );
        }
    }

    // STEP 2: Trim tail metadata down to the actual flash chip size, if enabled
    // (default: on, 8 MiB, measured AFTER the header strip above). Only ever
    // truncates from the end — the DMI region and all firmware volumes live
    // near the start of the (now header-stripped) image, well below any sane
    // chip-size boundary, so this is safe as long as --chip-size (or the
    // default) is >= the DMI --end offset, which cli::parse_args verifies
    // before touching anything.
    if let Some(target_size) = args.chip_size {
        for (label, buf) in [("old", &mut old_buf), ("new", &mut new_buf)] {
            match image::trim_to_chip_size(buf, target_size) {
                Trim::Truncated { original_len } => println!(
                    "  (trimming --{label}: {} -> {} bytes, removed {} bytes of tail metadata \
                     past chip size 0x{:X})",
                    original_len,
                    target_size,
                    original_len - target_size,
                    target_size
                ),
                Trim::Short => println!(
                    "  (note: --{label} is {} bytes, smaller than --chip-size 0x{:X} — leaving as-is, \
                     not padding)",
                    buf.len(),
                    target_size
                ),
                Trim::Exact => {}
            }
        }
    }

    let region_len = args.end - args.start + 1;

    image::ensure_covers(&old_buf, args.end, "old")?;
    image::ensure_covers(&new_buf, args.end, "new")?;

    let diff_bytes = count_differing(
        &old_buf[args.start..=args.end],
        &new_buf[args.start..=args.end],
    );

    println!("=== copydmi: DMI transplant ===");
    println!(
        "  old bios : {} ({} bytes)",
        args.old.display(),
        old_buf.len()
    );
    println!(
        "  new bios : {} ({} bytes)",
        args.new.display(),
        new_buf.len()
    );
    println!(
        "  DMI range: 0x{:06X} - 0x{:06X} ({} bytes)",
        args.start, args.end, region_len
    );
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
        println!(
            "  NOTE: DMI region is already identical between old and new — nothing to transplant."
        );
    }

    let preview_len = PREVIEW_LEN.min(region_len);

    if !args.quiet {
        println!("  -- preview (first 32 bytes of range) --");
        println!(
            "{}",
            hex_preview_line("old", &old_buf, args.start, preview_len)
        );
        println!(
            "{}",
            hex_preview_line("new (before patch)", &new_buf, args.start, preview_len)
        );
    }

    if args.verify_uefi {
        uefi_report("old bios", &old_buf);
        uefi_report("new bios (pre-patch)", &new_buf);
    }

    if args.show_dmi {
        dmi_report(&old_buf, &new_buf, None);
    }

    // Fail closed: without a real LENV DMI block in the old dump there is nothing
    // to preserve or verify, so a "successful" transplant would be meaningless.
    if !args.skip_dmi_check {
        require_old_dmi(&old_buf)?;
    }

    if args.dry_run {
        println!("  (dry-run) no output written.");
        return Ok(());
    }

    let mut patched = transplant(&old_buf, &new_buf, args.start, args.end);

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

    // Verify BEFORE writing: a patched image that lost/changed the old DMI is never
    // written to disk (and the process exits non-zero via the Err).
    if !args.skip_dmi_check {
        if args.show_dmi {
            dmi_report(&old_buf, &new_buf, Some(&patched));
        }
        verify_patched_dmi(&old_buf, &patched)?;
    }

    fs::write(&args.out, &patched)
        .map_err(|e| format!("failed to write --out '{}': {e}", args.out.display()))?;

    if !args.quiet {
        println!("  -- preview (first 32 bytes of range, after patch) --");
        println!(
            "{}",
            hex_preview_line("out", &patched, args.start, preview_len)
        );
    }

    if args.verify_uefi {
        uefi_report("patched output", &patched);
    }

    if args.show_dmi && args.skip_dmi_check {
        dmi_report(&old_buf, &new_buf, Some(&patched));
    }

    println!(
        "  wrote patched bios: {} ({} bytes)",
        args.out.display(),
        patched.len()
    );
    println!(
        "  DONE. Verify with a hex editor / diff before flashing to the chip. \
         This tool swaps the DMI byte range and (if --fix-uefi-checksums was passed) repairs \
         FFS-level data checksums for files overlapping that range. It does NOT validate FV \
         header checksums beyond reporting them, does NOT handle extended FFS headers, GUIDed/\
         compressed sections, Insyde vendor signing, or secure-boot keys."
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{parse_args, Command};
    use std::path::{Path, PathBuf};

    /// Per-test scratch directory under the system temp dir, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("copydmi-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn path(&self, file: &str) -> PathBuf {
            self.0.join(file)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn args(extra: &[&str], old: &Path, new: &Path, out: &Path) -> Args {
        let mut argv: Vec<String> = [
            "--old",
            old.to_str().unwrap(),
            "--new",
            new.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--quiet",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        argv.extend(extra.iter().map(|s| s.to_string()));
        match parse_args(argv) {
            Ok(Command::Run(a)) => a,
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn transplants_with_separate_old_and_new_header_sizes() {
        let s = Scratch::new("separate-headers");
        let (old, new, out) = (s.path("old.bin"), s.path("new.bin"), s.path("out.bin"));

        // old: 0x10-byte wrapper + 0x4000 image of 0xAA, tail junk.
        let mut old_img = vec![0xEEu8; 0x10];
        old_img.extend(vec![0xAAu8; 0x4000]);
        old_img.extend(vec![0xEEu8; 0x100]);
        // new: 0x20-byte wrapper + 0x4000 image of 0x11.
        let mut new_img = vec![0xEEu8; 0x20];
        new_img.extend(vec![0x11u8; 0x4000]);
        fs::write(&old, &old_img).unwrap();
        fs::write(&new, &new_img).unwrap();

        let a = args(
            &[
                "--old-header-size",
                "0x10",
                "--header-size",
                "0x20",
                "--chip-size",
                "0x4000",
                "--skip-dmi-check",
            ],
            &old,
            &new,
            &out,
        );
        run(&a).unwrap();

        let got = fs::read(&out).unwrap();
        assert_eq!(got.len(), 0x4000);
        assert!(got[..0x1000].iter().all(|&b| b == 0x11));
        assert!(got[0x1000..=0x2FFF].iter().all(|&b| b == 0xAA));
        assert!(got[0x3000..].iter().all(|&b| b == 0x11));
    }

    #[test]
    fn refuses_existing_output_without_force_and_dry_run_writes_nothing() {
        let s = Scratch::new("safety");
        let (old, new, out) = (s.path("old.bin"), s.path("new.bin"), s.path("out.bin"));
        fs::write(&old, vec![1u8; 0x4000]).unwrap();
        fs::write(&new, vec![2u8; 0x4000]).unwrap();

        let a = args(
            &["--no-trim", "--dry-run", "--skip-dmi-check"],
            &old,
            &new,
            &out,
        );
        run(&a).unwrap();
        assert!(!out.exists());

        fs::write(&out, b"keep").unwrap();
        let a = args(&["--no-trim", "--skip-dmi-check"], &old, &new, &out);
        let err = run(&a).unwrap_err();
        assert!(err.starts_with("output file '"), "{err}");
        assert_eq!(fs::read(&out).unwrap(), b"keep");

        let a = args(
            &["--no-trim", "--force", "--skip-dmi-check"],
            &old,
            &new,
            &out,
        );
        run(&a).unwrap();
        assert_eq!(fs::read(&out).unwrap().len(), 0x4000);
    }

    #[test]
    fn rejects_inputs_too_small_for_the_dmi_range() {
        let s = Scratch::new("too-small");
        let (old, new, out) = (s.path("old.bin"), s.path("new.bin"), s.path("out.bin"));
        fs::write(&old, vec![0u8; 0x2FFF]).unwrap();
        fs::write(&new, vec![0u8; 0x4000]).unwrap();
        let err = run(&args(&["--no-trim", "--skip-dmi-check"], &old, &new, &out)).unwrap_err();
        assert_eq!(
            err,
            "old bios file is too small (12287 bytes) for DMI range ending at 0x2FFF"
        );
        assert!(!out.exists());
    }

    // ---- fail-closed DMI behaviour (regression tests for the (None, None) => MATCH bug) ----

    use crate::lenv::testutil::build_block;

    /// 0x4000-byte image: 0xFF fill, optional LENV blocks at 0x1000 / 0x2000.
    fn image_with(b1: Option<&[u8]>, b2: Option<&[u8]>) -> Vec<u8> {
        let mut img = vec![0xFFu8; 0x4000];
        if let Some(b) = b1 {
            img[0x1000..0x2000].copy_from_slice(b);
        }
        if let Some(b) = b2 {
            img[0x2000..0x3000].copy_from_slice(b);
        }
        img
    }

    fn write_pair(s: &Scratch, old: &[u8], new: &[u8]) -> (PathBuf, PathBuf, PathBuf) {
        let (o, n, out) = (s.path("old.bin"), s.path("new.bin"), s.path("out.bin"));
        fs::write(&o, old).unwrap();
        fs::write(&n, new).unwrap();
        (o, n, out)
    }

    const SAFE: [&str; 3] = ["--no-trim", "--show-dmi", "--force"];

    #[test]
    fn refuses_when_old_dump_has_no_lenv_and_writes_nothing() {
        // The exact scenario from the bug report: no LENV on either side.
        let s = Scratch::new("no-lenv-both");
        let blank = image_with(None, None);
        let (o, n, out) = write_pair(&s, &blank, &blank);
        let err = run(&args(&SAFE, &o, &n, &out)).unwrap_err();
        assert!(err.contains("no usable LENV DMI block"), "{err}");
        assert!(!out.exists(), "no output may be written");
    }

    #[test]
    fn refuses_when_old_has_no_lenv_even_if_new_has_one() {
        let s = Scratch::new("no-lenv-old");
        let good = build_block(2, 0x5A, &[(0x0400, b"PF343TLP\0")]);
        let (o, n, out) = write_pair(
            &s,
            &image_with(None, None),
            &image_with(Some(&good), Some(&good)),
        );
        assert!(run(&args(&SAFE, &o, &n, &out)).is_err());
        assert!(!out.exists());
    }

    #[test]
    fn refuses_when_old_lenv_has_zero_entries() {
        let s = Scratch::new("empty-lenv");
        let empty = build_block(1, 0x11, &[]);
        let (o, n, out) = write_pair(
            &s,
            &image_with(Some(&empty), Some(&empty)),
            &image_with(None, None),
        );
        assert!(run(&args(&SAFE, &o, &n, &out)).is_err());
        assert!(!out.exists());
    }

    #[test]
    fn succeeds_and_preserves_old_dmi_in_output() {
        let s = Scratch::new("happy");
        let mine = build_block(
            5,
            0x5A,
            &[(0x0400, b"PF343TLP\0"), (0x0200, b"82C600ERID\0")],
        );
        let stock = build_block(1, 0x33, &[(0x0400, b"STOCKSN\0")]);
        let (o, n, out) = write_pair(
            &s,
            &image_with(Some(&mine), Some(&mine)),
            &image_with(Some(&stock), Some(&stock)),
        );
        run(&args(&SAFE, &o, &n, &out)).unwrap();
        let got = fs::read(&out).unwrap();
        assert_eq!(&got[0x1000..0x2000], &mine[..]);
        assert_eq!(&got[0x2000..0x3000], &mine[..]);
        assert_eq!(got.len(), 0x4000);
    }

    #[test]
    fn succeeds_with_only_one_usable_old_block() {
        // A single healthy redundant block is enough; the blank one is NO DATA, not a failure.
        let s = Scratch::new("one-block");
        let mine = build_block(5, 0x5A, &[(0x0400, b"PF343TLP\0")]);
        let (o, n, out) = write_pair(&s, &image_with(Some(&mine), None), &image_with(None, None));
        run(&args(&SAFE, &o, &n, &out)).unwrap();
        assert_eq!(&fs::read(&out).unwrap()[0x1000..0x2000], &mine[..]);
    }

    #[test]
    fn dmi_verification_runs_without_show_dmi_flag() {
        // Verification must not depend on the optional --show-dmi display flag.
        let s = Scratch::new("no-flag");
        let blank = image_with(None, None);
        let (o, n, out) = write_pair(&s, &blank, &blank);
        assert!(run(&args(&["--no-trim", "--force"], &o, &n, &out)).is_err());
        assert!(!out.exists());
    }

    #[test]
    fn range_not_covering_blocks_fails_postcheck_and_writes_nothing() {
        // --start/--end that skip block 2 leave the NEW image's (different) block 2 in place:
        // the patched output no longer carries old DMI -> must be rejected, not written.
        let s = Scratch::new("partial-range");
        let mine = build_block(5, 0x5A, &[(0x0400, b"PF343TLP\0")]);
        let stock = build_block(1, 0x33, &[(0x0400, b"STOCKSN\0")]);
        let (o, n, out) = write_pair(
            &s,
            &image_with(Some(&mine), Some(&mine)),
            &image_with(Some(&stock), Some(&stock)),
        );
        let a = args(
            &["--no-trim", "--start", "0x1000", "--end", "0x1FFF"],
            &o,
            &n,
            &out,
        );
        let err = run(&a).unwrap_err();
        assert!(
            err.contains("0x2000") && err.contains("DO NOT FLASH"),
            "{err}"
        );
        assert!(!out.exists());
    }

    #[test]
    fn skip_dmi_check_allows_non_lenv_models() {
        let s = Scratch::new("skip");
        let blank = image_with(None, None);
        let (o, n, out) = write_pair(&s, &blank, &blank);
        run(&args(&["--no-trim", "--skip-dmi-check"], &o, &n, &out)).unwrap();
        assert!(out.exists());
    }

    #[test]
    fn dry_run_also_rejects_old_without_lenv() {
        let s = Scratch::new("dry");
        let blank = image_with(None, None);
        let (o, n, out) = write_pair(&s, &blank, &blank);
        assert!(run(&args(&["--no-trim", "--dry-run"], &o, &n, &out)).is_err());
    }
}
