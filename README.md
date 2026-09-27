# copydmi (Rust) — Lenovo DMI/SMBIOS Transplant and Clean-BIOS Tool

**English** | [Bahasa Indonesia](README.id.md)

[![CI](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

`copydmi` is a cross-platform Rust CLI for restoring Lenovo machine-specific DMI/SMBIOS data from an original BIOS dump into a clean or newer Lenovo firmware image. It automates the community "CopyDMI" workflow usually performed manually with Tiny Hexer macros, then produces a raw `.bin` image ready for an external SPI programmer such as a CH341A.

It can:

- extract Lenovo Inno Setup BIOS installers (`.exe`) in memory;
- remove an installer capsule/wrapper header and trim output to the physical flash-chip size;
- handle different wrapper sizes for the old dump and new firmware;
- decode Lenovo InsydeH2O `LENV` DMI storage with `--show-dmi`;
- compare old and patched DMI blocks and emit an explicit `MATCH` / `MISMATCH` verdict;
- scan UEFI Firmware Volumes and repair applicable overlapping FFS data checksums.

> **Warning:** BIOS flashing can permanently brick a device. Make several identical backups of the original chip, verify their SHA-256 hashes, and validate offsets for the exact Lenovo model/family before programming anything.

## Why this exists

Community `CopyDMI.zip` / `CopyDMI.mps` attachments are proprietary Tiny Hexer macro files, sometimes unavailable without a forum account or post count. This project reimplements the byte-range transplant as an auditable CLI and automates the otherwise manual clean-BIOS step.

No Windows, Tiny Hexer, HxD, 7-Zip, `innoextract`, or Docker is required to use an official Lenovo installer as new firmware input.

## Install

### Linux / macOS

```bash
curl -fsSL https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.sh | bash
```

### Windows PowerShell

```powershell
irm https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.ps1 | iex
```

The scripts download the latest GitHub Release, verify SHA-256, and install to the user PATH (`~/.local/bin` on Linux; `%LOCALAPPDATA%\Programs\copydmi` on Windows). Rust/Cargo is not required on the target computer.

Open a new terminal after installation, then run:

```bash
copydmi --help
```

## Build from source

With a local Rust toolchain:

```bash
cargo build --release
```

Or with Docker:

```bash
docker run --rm -v $(pwd):/app -w /app rust:latest cargo build --release
```

The binary is created at `target/release/copydmi`.

## Quick start

```bash
copydmi \
  --old original_chip_dump.bin \
  --new e8cn41ww.exe \
  --out bios_flash_ready.bin \
  --fix-uefi-checksums \
  --show-dmi
```

`--old` and `--new` accept either:

- an official Lenovo `.exe` Inno Setup installer, extracted in memory; or
- a raw firmware/dump file (`.bin`, `.cap`, `.fd`, `.rom`).

For the validated Lenovo ADA-series layout (V14-ADA, V15-ADA, IdeaPad 3 14/15/17ADA05; 82C7/E8CN family), defaults do the following:

1. strip the 792-byte (`0x318`) wrapper from the **new** installer image;
2. trim it to the 8 MiB physical flash capacity (`0x800000` / 8,388,608 bytes);
3. transplant DMI `0x1000-0x2FFF`, relative to the clean firmware image;
4. optionally repair required FFS data checksums overlapping that range.

The resulting `--out` is a raw 8 MiB image ready to program to a compatible SPI flash chip. It is **not** an untrimmed capsule or installer artifact.

## Validated ADA-series clean-BIOS layout

Official Lenovo ADA installer payloads such as `e8cn39ww.exe` and `e8cn41ww.exe` contain more than the physical flash image. The extracted payload is 8,950,768 bytes, while a W25Q64-class chip is 8 MiB / 8,388,608 bytes.

| Raw-file offset | Length | Content |
|---|---:|---|
| `0x000` | 80 bytes | `EFI_CAPSULE_HEADER` |
| `0x050` | 72 bytes | outer Firmware Volume wrapper |
| `0x098` | 640 bytes | FFS header plus signature/crypto blob |
| **`0x318`** | **8,388,608 bytes** | **actual flashable firmware image** |
| `0x800318` | 562,152 bytes | debug strings, PDB paths, installer metadata |

The `0x318` base was verified against two official images. The clean image contains coherent valid Firmware Volumes at `0x310000`, `0x360000`, `0x390000`, `0x3A0000`, and `0x730000`, ending exactly at `0x800000`.

A previous implementation wrote at `0x1000` of the unstripped file, which was an `FF FF FF...` area rather than the DMI region. `copydmi` strips the wrapper first, then applies DMI offsets relative to the clean image.

## CLI options

```text
--start <HEX|DEC>       DMI start offset relative to clean firmware (default: 0x1000)
--end <HEX|DEC>         Inclusive DMI end offset (default: 0x2FFF)
--header-size <HEX|DEC> Bytes stripped from the front of --new before other operations
                        (default: 0x318 / 792 bytes for validated ADA-series images)
--old-header-size <N>   Bytes stripped from the front of --old (default: same as --header-size).
                        Use 0 when --old is an already-clean chip dump that starts with LDBG,
                        while --new still needs its installer wrapper removed.
--chip-size <HEX|DEC>   Target flash-chip size after stripping (default: 0x800000 / 8 MiB)
--no-trim               Disable header stripping and chip-size trimming
--force                 Overwrite an existing --out file
--dry-run               Validate and show a preview without writing output
--quiet                 Suppress hex previews
--verify-uefi           Scan Firmware Volumes / FFS and report checksum state
--fix-uefi-checksums    Repair applicable FFS data checksums overlapping the transplant range
--show-dmi              Decode and print LENV blocks from old/new/patched buffers; after a real
                        transplant, print MATCH or MISMATCH for every redundant DMI block.
```

### Different old/new wrappers

A raw chip dump may already be clean (`LDBG` at offset `0x0`) while the new firmware comes from a Lenovo installer and still needs `0x318` removed. Do not strip both sides equally:

```bash
copydmi \
  --old original_clean_chip_dump.bin \
  --new e8cn41ww.exe \
  --out bios_flash_ready.bin \
  --old-header-size 0 \
  --show-dmi \
  --fix-uefi-checksums
```

Without `--old-header-size 0`, the old dump is stripped a second time, DMI offsets shift, and the transplant copies the wrong bytes.

### Other Lenovo families

Offsets and wrappers are **not universal** across Lenovo models. Override values only after validating the exact family/model layout:

```bash
copydmi \
  --old old.bin \
  --new new.bin \
  --out out.bin \
  --header-size 0 \
  --start 0x520000 \
  --end 0x5207FF \
  --no-trim
```

## Recommended workflow

1. Read the physical chip with a CH341A or equivalent programmer and preserve the untouched original dump.
2. Read the chip at least two more times without moving the clip; all SHA-256 hashes must match.
3. Download official firmware for the exact machine family.
4. Generate output with `--show-dmi` and, if needed, `--old-header-size 0`.
5. Confirm the output size matches the physical chip and `--show-dmi` reports `MATCH` for both DMI blocks. A `MISMATCH` means **do not flash**.
6. Write with `flashrom` or another verified SPI programming tool.
7. Read the chip back and compare its SHA-256 against the exact output file.
8. After reassembly, load BIOS setup defaults before booting the OS.

## Flashing with CH341A and flashrom on Linux

This procedure was verified end-to-end on a Lenovo V15-ADA (`82C7A00RVN`) with a Winbond W25Q64.W 8 MiB chip: `flashrom` write verification passed and a separate readback matched the intended image SHA-256.

### 1. Install flashrom

On Arch/CachyOS, flashrom is normally in the official repositories; AUR/yay is not needed:

```bash
sudo pacman -S flashrom
```

### 2. Check the programmer

```bash
lsusb | grep -i "1a86:5512"
```

Expected text includes `QinHeng Electronics CH341 in EPP/MEM/I2C mode`.

### 3. Connect safely

Remove the BIOS chip from the motherboard, then attach the SOIC-8 clip/programmer. Do not program an in-circuit motherboard unless you have verified that its rails cannot conflict with the programmer.

### 4. Detect the chip

```bash
sudo flashrom -p ch341a_spi
```

Example:

```text
Found Winbond flash chip "W25Q64.W" (8192 kB, SPI) on ch341a_spi.
```

The detected capacity must match the generated output. `8192 kB` equals 8,388,608 bytes / 8 MiB. If flashrom reports `No EEPROM/flash device found`, reseat the clip; do not write.

### 5. Back up and prove read reliability

```bash
sudo flashrom -p ch341a_spi -r backup_chip_original_$(date +%Y%m%d).bin
sudo flashrom -p ch341a_spi -r read2.bin
sudo flashrom -p ch341a_spi -r read3.bin
sha256sum backup_chip_original_*.bin read2.bin read3.bin
```

All hashes must be identical. Different hashes mean the clip/programmer connection is unreliable.

### 6. Verify the output before writing

```bash
sha256sum bios_flash_ready.bin
```

Record this hash for the post-write readback comparison.

### 7. Write and verify

```bash
sudo flashrom -p ch341a_spi -w bios_flash_ready.bin
```

Do not move the clip during erase/write/verify. A successful operation ends with:

```text
Verifying flash... VERIFIED.
```

### 8. Independently read back the chip

```bash
sudo flashrom -p ch341a_spi -r verify_final.bin
sha256sum verify_final.bin bios_flash_ready.bin
```

The two hashes must match exactly. Only then reinstall the chip, observing pin-1 orientation.

### 9. First boot

Enter BIOS setup (often `F2` on Lenovo), choose **Load Setup Defaults** / **Load Optimized Defaults**, save, and reboot. Confirm the serial number and machine type when the firmware information page exposes them.

If an old Windows installation repeatedly enters Automatic Repair, a clean OS installation may be more reliable than attempting to repair the old installation.

## Limits and safety boundaries

- The tool copies a selected byte range and can repair applicable overlapping **FFS-level** data checksums. It does not validate every vendor signature, GUIDed/compressed section, secure-boot key, extended FFS header, or platform-specific Insyde integrity mechanism.
- Default offsets, wrapper size, and chip size are verified for the ADA-series family above, not every Lenovo laptop.
- `--show-dmi` decodes only blocks whose `LENV` signature and entry structure pass sanity checks. Unknown entry types are shown as raw hex rather than guessed labels.
- Never overwrite the only original raw-chip backup.

## UEFI checksum support

`--verify-uefi` and `--fix-uefi-checksums` parse standard UEFI Firmware Volume (FV) and Firmware File System (FFS) structures:

1. FV headers are found through `_FVH` and validated with the 16-bit header checksum.
2. Classic 24-byte FFS headers and their integrity headers are checked.
3. FFS data checksums apply only when `FFS_ATTRIB_CHECKSUM` (`0x40`) is set. Otherwise the integrity byte is conventionally fixed at `0xAA`.
4. `--fix-uefi-checksums` updates applicable checksummed FFS data regions that overlap the transplanted DMI range.

## Research and implementation references

### Lenovo DMI / LENV (`--show-dmi`)

- https://github.com/Shmurkio/LenovoDMIDecryptor — primary reference for Lenovo `LENV` storage, XOR decrypt/encrypt, redundant blocks, entries, and the related `LDBG` change log. `copydmi` implements its own Rust decoder; it does not invoke that Windows binary.
- https://winraid.level1techs.com/t/lenovo-dmi-decryption-tool/98137 — community thread by the decryptor author.
- https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/3286884-lenovo-dmi-decrypter — independent community corroboration of Lenovo DMI decrypt/extract/transfer use cases.
- https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/102197-copy-dmi-info-easily-with-hex-editing-software-and-macro-script — original manual CopyDMI guide; its `0x520000-0x5207FF` range is generic, not a universal default.
- https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/85892-ideapad-3-14ada05-15ada05-17ada05-lenovo-v14-ada-v15-ada-bios — ADA-series community thread used during initial `0x1000-0x2FFF` research.
- https://winraid.level1techs.com/t/problem-bad-lenovo-legion-5-pro-bios-flash/39904 — independent evidence of related Lenovo InsydeH2O `LDBG` / `LENV` machine-specific regions.

### Firmware container and UEFI structures

- https://github.com/LongSoft/InsydeImageExtractor/blob/master/extractor.c — Insyde image extraction reference. The 82C7 wrapper is independently byte-validated because it differs from layouts handled by that extractor.
- https://uefi.org/sites/default/files/resources/UEFI_PI_Spec_Final_Draft_1.9.pdf — UEFI Platform Initialization specification, used for FV/FFS parsing and checksum handling.
- Independent byte-level validation of official `e8cn39ww.exe` and `e8cn41ww.exe`: clean firmware starts at `0x318` and is exactly `0x800000` bytes. The recorded clean firmware SHA-256 is `28049cb8efd57c04a8f879f2bdc66bd43b4ceae2a8c5f231369f3baf80f19c02`.

### Hardware flashing

- https://flashrom.org/supported_hw/supported_prog/ch341ab.html — upstream CH341A/B programmer documentation.
- https://flashrom.org/classic_cli_manpage.html — upstream flashrom CLI documentation, including `ch341a_spi`, read, write, and verification operations.
- https://winraid.level1techs.com/t/guide-how-to-use-a-ch341a-spi-programmer-flasher/33041 — additional community CH341A/SPI guide.

## License

MIT. See [LICENSE](LICENSE).
