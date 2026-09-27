//! The copydmi pipeline: load → strip header → trim to chip size → report →
//! transplant → optional UEFI checksum fixup → write → verify.

use std::fs;

use crate::cli::Args;
use crate::image::{self, Trim};
use crate::lenv::dmi_report;
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

    if args.show_dmi {
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

        let a = args(&["--no-trim", "--dry-run"], &old, &new, &out);
        run(&a).unwrap();
        assert!(!out.exists());

        fs::write(&out, b"keep").unwrap();
        let a = args(&["--no-trim"], &old, &new, &out);
        let err = run(&a).unwrap_err();
        assert!(err.starts_with("output file '"), "{err}");
        assert_eq!(fs::read(&out).unwrap(), b"keep");

        let a = args(&["--no-trim", "--force"], &old, &new, &out);
        run(&a).unwrap();
        assert_eq!(fs::read(&out).unwrap().len(), 0x4000);
    }

    #[test]
    fn rejects_inputs_too_small_for_the_dmi_range() {
        let s = Scratch::new("too-small");
        let (old, new, out) = (s.path("old.bin"), s.path("new.bin"), s.path("out.bin"));
        fs::write(&old, vec![0u8; 0x2FFF]).unwrap();
        fs::write(&new, vec![0u8; 0x4000]).unwrap();
        let err = run(&args(&["--no-trim"], &old, &new, &out)).unwrap_err();
        assert_eq!(
            err,
            "old bios file is too small (12287 bytes) for DMI range ending at 0x2FFF"
        );
        assert!(!out.exists());
    }
}
