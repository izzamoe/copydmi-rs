# Technical notes

[← Back to README](../README.md)

## Why this exists

Community `CopyDMI.zip` / `CopyDMI.mps` attachments are proprietary Tiny Hexer macro files, sometimes unavailable without a forum account or post count. copydmi reimplements the byte-range transplant as an auditable CLI and automates the otherwise manual clean-BIOS step. No Windows, Tiny Hexer, HxD, 7-Zip, `innoextract` or Docker is needed to use an official Lenovo installer as new firmware input.

## ADA-series clean-BIOS layout (validated)

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

## UEFI checksum support

`--verify-uefi` and `--fix-uefi-checksums` parse standard UEFI Firmware Volume (FV) and Firmware File System (FFS) structures:

1. FV headers are found through `_FVH` and validated with the 16-bit header checksum.
2. Classic 24-byte FFS headers and their integrity headers are checked.
3. FFS data checksums apply only when `FFS_ATTRIB_CHECKSUM` (`0x40`) is set. Otherwise the integrity byte is conventionally fixed at `0xAA`.
4. `--fix-uefi-checksums` updates applicable checksummed FFS data regions that overlap the transplanted DMI range.

## LENV verification (v0.4.1+)

Two redundant LENV blocks live at `0x1000` and `0x2000` of the clean image. Each is 0x1000 bytes: a 16-byte plain header (`LENV`, generation, entry count, access flag, XOR key, additive 16-bit checksum) and an XOR-encrypted body of entries.

copydmi checks, with no flag needed:

1. **Before anything is written:** `--old` has at least one block with decodable entries.
2. **Before the output is written:** every usable block in `--old` appears byte-for-byte (type + data, same order) in the patched image.

A block that is absent or empty in `--old` is `NO DATA`: nothing to compare, and never a pass on its own. If neither block is usable the run fails.
