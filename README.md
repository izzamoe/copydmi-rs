# copydmi

**English** | [Bahasa Indonesia](README.id.md)

[![CI](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

Restore a Lenovo laptop's serial number, UUID and model (DMI/SMBIOS) after a BIOS chip was erased or replaced. `copydmi` takes your original chip dump plus an official Lenovo BIOS and produces a raw image ready for a CH341A / `flashrom`.

> **Warning:** BIOS flashing can permanently brick a device. Keep several identical backups of the original chip, check their SHA-256 hashes, and confirm offsets for your exact Lenovo model before writing anything.

## What it does

- Reads an official Lenovo `.exe` installer directly (extracted in memory, no 7-Zip or Docker) or a raw `.bin` / `.cap` / `.fd` / `.rom`.
- Strips the installer wrapper and trims to the physical chip size.
- Copies the DMI region from your old dump into the new firmware.
- Decodes the Lenovo `LENV` DMI blocks and **refuses to write** an image that does not carry your DMI through unchanged.
- Optionally repairs UEFI FFS checksums overlapping the DMI range.

**Use it for:** Lenovo InsydeH2O machines with `LENV` DMI storage. Defaults are validated for the ADA series (V14-ADA, V15-ADA, IdeaPad 3 14/15/17ADA05, E8CN BIOS family).
**Do not use it blindly for:** other Lenovo families or other brands. Offsets and wrapper sizes differ, see [Other models](#other-models).

## Quick start

1. Install ([details](#install)):

   ```bash
   curl -fsSL https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.sh | bash
   ```

2. Run it with your original dump and the official installer:

   ```bash
   copydmi --old original_chip_dump.bin --new e8cn41ww.exe --out bios_flash_ready.bin \
     --fix-uefi-checksums --show-dmi
   ```

   If the dump is a raw chip read that already starts with `LDBG`, add `--old-header-size 0` ([why](#scenarios)).

3. Check the result ([how to read it](#reading-the-result)). You want `MATCH` on the blocks, exit code `0`, and an output of exactly the chip size (8,388,608 bytes on ADA series).

4. Flash and verify: [docs/flashing.md](docs/flashing.md).

Example output of a successful run (synthetic test dumps, not a real machine):

```text
-- verdict --
  block @0x1000: MATCH — patched output carries the real old-bios DMI values unchanged
  block @0x2000: MATCH — patched output carries the real old-bios DMI values unchanged
  wrote patched bios: bios_flash_ready.bin (8388608 bytes)
```

Example of a refused run (old dump has no DMI at all), exit code 1, no file written:

```text
error: old bios has no usable LENV DMI block at 0x1000 or 0x2000 (wrong dump, wrong --old-header-size, ...
```

## Install

| Platform | Command |
|---|---|
| Linux / macOS | `curl -fsSL https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.sh \| bash` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.ps1 \| iex` |
| Manual | Download from [Releases](https://github.com/izzamoe/copydmi-rs/releases) and check `SHA256SUMS.txt` |
| From source | `cargo build --release` (binary at `target/release/copydmi`) |

The scripts download the latest release, verify SHA-256 and install to the user PATH (`~/.local/bin` or `%LOCALAPPDATA%\Programs\copydmi`). Rust is not needed on the target machine. Open a new terminal, then run `copydmi --help`.

## Reading the result

Per DMI block (`0x1000` and `0x2000`):

| Verdict | Meaning |
|---|---|
| `MATCH` | Old and patched blocks hold identical entries. |
| `MISMATCH` | Patched output lost or changed your DMI. Error, nothing written. **Do not flash.** |
| `NO DATA` | Old dump has nothing usable in this block, so nothing to compare. Never counts as a pass. |

| Exit code | Meaning |
|---|---|
| `0` | Output written, every usable old block verified. |
| `1` | Refused or failed: no usable LENV in `--old`, verification mismatch, output exists without `--force`, size or range error. No output file is written for DMI failures. |

The LENV check is always on (since v0.4.1) and does not depend on `--show-dmi`; that flag only prints the decoded fields (serial, machine type, UUID). Use `--skip-dmi-check` only for models that do not store DMI as LENV.

## Options

| Option | Default | Purpose |
|---|---|---|
| `--old <FILE>` | required | Original dump with your real DMI (`.bin` or Lenovo `.exe`). |
| `--new <FILE>` | required | Clean official firmware (`.bin` or Lenovo `.exe`). |
| `--out <FILE>` | required | Output path. Inputs are never overwritten. |
| `--start <N>` / `--end <N>` | `0x1000` / `0x2FFF` | Inclusive DMI range, relative to the clean image. |
| `--header-size <N>` | `0x318` | Bytes stripped from the front of `--new`. |
| `--old-header-size <N>` | same as `--header-size` | Bytes stripped from the front of `--old`. Use `0` for an already-clean chip dump. |
| `--chip-size <N>` | `0x800000` (8 MiB) | Trim to this size after stripping. |
| `--no-trim` | off | Disable header stripping and trimming. |
| `--show-dmi` | off | Print decoded LENV fields and per-block verdicts. |
| `--skip-dmi-check` | off | Disable the fail-closed LENV check (non-LENV models only). |
| `--verify-uefi` | off | Report FV/FFS checksum state. |
| `--fix-uefi-checksums` | off | Repair applicable FFS data checksums overlapping the DMI range (implies `--verify-uefi`). |
| `--dry-run` | off | Validate and preview without writing. |
| `--quiet` | off | Suppress hex previews. |
| `--force` | off | Overwrite an existing `--out`. |

Numbers accept hex (`0x1000`) or decimal.

## Scenarios

**Standard ADA series.** Defaults apply: strip `0x318` from the installer, trim to 8 MiB, transplant `0x1000-0x2FFF`.

**Old dump is already clean.** A raw chip read starts with `LDBG` and has no installer wrapper, while the new firmware is still an installer. Do not strip both sides:

```bash
copydmi --old original_clean_chip_dump.bin --new e8cn41ww.exe --out bios_flash_ready.bin \
  --old-header-size 0 --show-dmi --fix-uefi-checksums
```

Without `--old-header-size 0` the old dump is stripped twice, offsets shift and the wrong bytes are copied.

### Other models

Offsets and wrappers are **not universal**. Override only after validating your exact family:

```bash
copydmi --old old.bin --new new.bin --out out.bin \
  --header-size 0 --start 0x520000 --end 0x5207FF --no-trim
```

## Recommended workflow

1. Read the physical chip with a CH341A and keep the untouched dump.
2. Read it at least twice more without moving the clip. All SHA-256 hashes must match.
3. Download official firmware for the exact machine family.
4. Generate the output with `--show-dmi` (add `--old-header-size 0` if needed).
5. Confirm `MATCH`, exit code `0` and the expected output size.
6. Write with `flashrom`, read back, and compare SHA-256 against the output file.
7. After reassembly, load BIOS setup defaults before booting the OS.

Full hardware procedure: [docs/flashing.md](docs/flashing.md).

## Limits and safety

- Copies a byte range and can repair applicable FFS-level data checksums. It does not validate every vendor signature, GUIDed or compressed section, secure-boot key, extended FFS header or Insyde integrity mechanism.
- Default offsets, wrapper size and chip size are verified for the ADA series only.
- `--show-dmi` decodes only blocks whose `LENV` signature and entry structure pass sanity checks; unknown entry types print as raw hex.
- Never overwrite the only original chip backup.

## More documentation

- [docs/flashing.md](docs/flashing.md): CH341A + flashrom on Linux, step by step.
- [docs/technical.md](docs/technical.md): ADA-series image layout, UEFI checksum support, LENV verification.
- [docs/references.md](docs/references.md): sources and prior work.
- [Releases](https://github.com/izzamoe/copydmi-rs/releases): binaries and changelogs.

## License

MIT. See [LICENSE](LICENSE).
