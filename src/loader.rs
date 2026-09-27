//! Flexible input loading: --old / --new accept EITHER a raw BIOS image
//! (.cap/.fd/.bin/.rom/anything) OR a Lenovo BIOS update installer .exe
//! (Inno Setup wrapper). When given a .exe, the firmware image is extracted
//! in-memory using the pure-Rust `inno` crate (same logic as the standalone
//! lenovo-bios-extract-rs tool) — no external innoextract/7z/Docker needed.

use std::fs;
use std::path::Path;

/// Firmware file extensions in preference order: .cap > .fd > .bin/.rom.
const FW_EXT_TIERS: &[&[&str]] = &[&["cap"], &["fd"], &["bin", "rom"]];
const FW_MIN_PLAUSIBLE_SIZE: u64 = 1024 * 1024; // 1 MiB floor, rejects install.bat etc

/// Preference tier (lower = better) of a file name's extension, or None if
/// it isn't a firmware-looking extension at all.
fn fw_ext_tier(name: &str) -> Option<usize> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    FW_EXT_TIERS
        .iter()
        .position(|tier| tier.contains(&ext.as_str()))
}

/// Best-so-far firmware candidate while scanning installer entries.
#[derive(Debug, Default)]
struct Selection {
    best: Option<(usize, u64, String)>, // (tier, size, name)
}

impl Selection {
    /// Consider one embedded file: too-small files and non-firmware
    /// extensions are ignored; a better tier wins, then a larger size.
    fn offer(&mut self, name: String, size: u64) {
        let Some(tier) = fw_ext_tier(&name) else {
            return;
        };
        if size < FW_MIN_PLAUSIBLE_SIZE {
            return;
        }
        let better = match &self.best {
            None => true,
            Some((best_tier, best_size, _)) => {
                tier < *best_tier || (tier == *best_tier && size > *best_size)
            }
        };
        if better {
            self.best = Some((tier, size, name));
        }
    }

    fn into_name(self) -> Option<String> {
        self.best.map(|(_, _, name)| name)
    }
}

/// Extract the most plausible firmware image out of a Lenovo BIOS installer
/// .exe (Inno Setup archive), fully in-memory, no temp files written.
/// Same selection heuristic as lenovo-bios-extract-rs: prefer .cap > .fd >
/// .bin/.rom, pick the largest candidate >= 1 MiB within the best tier.
fn extract_firmware_from_exe(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path)
        .map_err(|e| format!("failed to open installer '{}': {e}", path.display()))?;
    let mut inno = inno::Inno::new(file).map_err(|e| {
        format!(
            "failed to parse '{}' as Inno Setup installer: {e}",
            path.display()
        )
    })?;

    println!(
        "  (detected .exe, extracting embedded firmware via pure-Rust Inno parser — Inno Setup version {})",
        inno.version()
    );

    // Pass 1: find best (tier, size) candidate name among embedded files.
    let mut selection = Selection::default();
    {
        let files = inno.filtered_files(|file_entry| {
            file_entry
                .file()
                .destination()
                .is_some_and(|d| fw_ext_tier(d).is_some())
        });
        for res in files {
            let (entry, data) = res.map_err(|e| format!("error reading installer entry: {e}"))?;
            let Some(dest) = entry.file().normalized_destination() else {
                continue;
            };
            if dest.is_empty() {
                continue;
            }
            selection.offer(dest, data.len() as u64);
        }
    }

    let Some(target_name) = selection.into_name() else {
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
            let Some(dest) = entry.file().normalized_destination() else {
                continue;
            };
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

/// True if `path` has a (case-insensitive) .exe extension.
fn is_exe(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

/// Load a BIOS image from disk. If the path ends in .exe, transparently
/// extract the embedded firmware image in-memory first (see
/// extract_firmware_from_exe). Otherwise the file is read as a raw BIOS
/// image directly (.cap/.fd/.bin/.rom/anything else).
pub fn load_bios_buf(path: &Path, label: &str) -> Result<Vec<u8>, String> {
    if is_exe(path) {
        extract_firmware_from_exe(path)
            .map_err(|e| format!("failed to extract firmware from --{label} installer: {e}"))
    } else {
        fs::read(path).map_err(|e| format!("failed to read --{label} '{}': {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn extension_tiers() {
        assert_eq!(fw_ext_tier("BIOS.cap"), Some(0));
        assert_eq!(fw_ext_tier("app/E8CN41WW.CAP"), Some(0));
        assert_eq!(fw_ext_tier("image.fd"), Some(1));
        assert_eq!(fw_ext_tier("x.bin"), Some(2));
        assert_eq!(fw_ext_tier("x.Rom"), Some(2));
        assert_eq!(fw_ext_tier("install.bat"), None);
        assert_eq!(fw_ext_tier("cap"), Some(0)); // no dot: whole name is the "extension"
        assert_eq!(fw_ext_tier(""), None);
    }

    #[test]
    fn selection_prefers_tier_then_size() {
        let mut s = Selection::default();
        s.offer("big.bin".into(), 16 * MIB);
        s.offer("small.fd".into(), 2 * MIB);
        assert_eq!(s.best.as_ref().unwrap().2, "small.fd");
        s.offer("a.cap".into(), 8 * MIB);
        s.offer("b.cap".into(), 9 * MIB);
        s.offer("c.cap".into(), 9 * MIB); // tie keeps the first seen
        s.offer("d.fd".into(), 32 * MIB);
        assert_eq!(s.into_name().as_deref(), Some("b.cap"));
    }

    #[test]
    fn selection_ignores_small_and_non_firmware_files() {
        let mut s = Selection::default();
        s.offer("tiny.cap".into(), MIB - 1);
        s.offer("setup.exe".into(), 64 * MIB);
        assert_eq!(s.into_name(), None);

        let mut s = Selection::default();
        s.offer("exact.rom".into(), MIB);
        assert_eq!(s.into_name().as_deref(), Some("exact.rom"));
    }

    #[test]
    fn exe_detection_is_case_insensitive() {
        assert!(is_exe(Path::new("e8cn41ww.exe")));
        assert!(is_exe(Path::new("dir/E8CN41WW.EXE")));
        assert!(!is_exe(Path::new("bios.bin")));
        assert!(!is_exe(Path::new("exe")));
        assert!(!is_exe(Path::new("bios.exe.bin")));
    }

    #[test]
    fn missing_raw_file_reports_label_and_path() {
        let err = load_bios_buf(Path::new("/nonexistent/copydmi/old.bin"), "old").unwrap_err();
        assert!(
            err.starts_with("failed to read --old '/nonexistent/copydmi/old.bin': "),
            "{err}"
        );
    }

    #[test]
    fn missing_installer_reports_label_and_path() {
        let err = load_bios_buf(Path::new("/nonexistent/copydmi/new.exe"), "new").unwrap_err();
        assert!(
            err.starts_with(
                "failed to extract firmware from --new installer: failed to open installer \
                 '/nonexistent/copydmi/new.exe': "
            ),
            "{err}"
        );
    }
}
