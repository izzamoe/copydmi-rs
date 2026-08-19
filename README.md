# copydmi (Rust) — DMI/SMBIOS Transplant Tool untuk BIOS Lenovo

[![CI](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

CLI Rust yang meng-otomatisasi proses "CopyDMI" ala komunitas badcaps.net (biasanya dikerjakan manual pakai Tiny Hexer + macro `.mps`), khusus buat transplant blok DMI (serial number, model number, UUID) dari dump BIOS lama lo ke file BIOS baru yang bersih — sebelum di-flash ulang ke chip pakai CH341A.

## Kenapa dibikin
Attachment `CopyDMI.zip`/`CopyDMI.mps` di forum itu file macro proprietary buat software Windows (Tiny Hexer), sering ke-lock di balik requirement post-count/premium di forum. Tool ini reimplementasi logic yang sama (copy byte range tertentu dari file A ke file B) sebagai CLI portable, auditable, cross-platform — ga perlu Windows/Tiny Hexer.

## Install (gampang, tinggal 1 command)

**Linux/macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.ps1 | iex
```

Script ini otomatis: download binary terbaru dari [GitHub Releases](https://github.com/izzamoe/copydmi-rs/releases), verifikasi checksum SHA-256, install ke PATH user (`~/.local/bin` di Linux, `%LOCALAPPDATA%\Programs\copydmi` di Windows). Ga perlu install Rust/Cargo di komputer target.

Setelah install, tinggal jalankan `copydmi --help` (buka terminal baru dulu kalau baru pertama install).

## Build dari source
Butuh Rust toolchain. Kalau ga ada di sistem lo, paling gampang pakai Docker:
```bash
docker run --rm -v $(pwd):/app -w /app rust:latest cargo build --release
```
Atau kalau ada Rust lokal:
```bash
cargo build --release
```
Binary hasil ada di `target/release/copydmi` (juga sudah dicopy ke `./copydmi` di folder ini).

## Pakai
```bash
./copydmi --old dump_asli_lo.bin --new e8cn39ww_extracted.bin --out bios_siap_flash.bin
```

**BARU — `--new` (dan `--old`) sekarang bisa langsung terima file `.exe` installer resmi Lenovo**, ga perlu extract manual dulu:
```bash
./copydmi --old dump_asli_lo.bin --new e8cn39ww.exe --out bios_siap_flash.bin
```
Kalau path berakhiran `.exe`, tool otomatis extract firmware image (`.cap`/`.fd`/`.bin`) dari dalamnya secara in-memory — pure Rust, pakai crate `inno`, ga butuh `innoextract`/`7z`/Docker sama sekali. Logic pemilihan file sama kayak `lenovo-bios-extract-rs`: prioritas `.cap` > `.fd` > `.bin`/`.rom`, ambil yang terbesar ≥1MB.

Default offset DMI: `0x1000` - `0x2FFF` — ini spesifik buat **Lenovo ADA-series (V14-ADA/V15-ADA/82C7, IdeaPad 3 14/15/17ADA05, chip E8CN)**, sesuai info dari thread komunitas badcaps.net khusus model ini.

Kalau model lo beda / offset beda, override manual:
```bash
./copydmi --old old.bin --new new.bin --out out.bin --start 0x520000 --end 0x5207FF
```

### Opsi lain
- `--dry-run` — cek dulu tanpa nulis file (liat berapa byte yang beda, preview hex)
- `--force` — timpa file `--out` kalau udah ada
- `--quiet` — skip preview hex

## Alur pemakaian lengkap (kasus Izzam - 82C7)
1. Dump chip BIOS lo yang sekarang pakai CH341A (walau corrupt) → simpan sebagai `dump_asli.bin`
2. Download BIOS resmi dari Lenovo (`e8cn39ww.exe` / `e8cn41ww.exe`) — **ga perlu extract manual**, tinggal pakai langsung
3. Jalankan copydmi, kasih EXE-nya langsung sebagai `--new`:
   ```bash
   ./copydmi --old dump_asli.bin --new e8cn39ww.exe --out bios_final.bin --fix-uefi-checksums
   ```
4. Cek output log — pastikan "bytes differing" masuk akal (ga 0, ga 100%), size input/output BIOS sama
5. Flash `bios_final.bin` ke chip pakai NeoProgrammer/flashrom via CH341A

(Kalau mau extract terpisah dulu buat inspeksi manual, tetap bisa pakai `../lenovo-bios-extract-rs/lenovo-bios-extract-rs e8cn39ww.exe bios_bersih.bin --list` dulu — tapi untuk alur normal ga perlu lagi, copydmi udah handle sendiri.)

## PENTING — Batasan tool ini
- Cuma swap byte range DMI + (opsional) fix checksum FFS-level yang overlap range itu. TIDAK menangani extended FFS header, GUIDed/compressed section internals, Insyde vendor signing, atau secure-boot key.
- Offset default (`0x1000-0x2FFF`) itu klaim dari satu thread komunitas untuk model spesifik ini — **selalu cross-check** dengan sumber lain / hasil compare manual sebelum percaya penuh, terutama kalau versi BIOS baru lo beda jauh dari versi yang dipakai forum waktu nentuin offset itu.
- Selalu simpan backup `dump_asli.bin` yang ORIGINAL (belum diapa-apain) terpisah, jangan overwrite.

## Fitur checksum UEFI (--verify-uefi / --fix-uefi-checksums)

Tool ini sekarang benar-benar mem-parsing struktur **UEFI Firmware Volume (FV)** dan **Firmware File System (FFS)** sesuai spec resmi (UEFI PI Volume 3) — bukan asumsi/tebakan:

```bash
./copydmi --old dump_asli.bin --new bios_bersih.bin --out bios_final.bin --fix-uefi-checksums
```

**Apa yang dicek:**
1. **FV header checksum** — scan seluruh file cari signature `_FVH`, validasi checksum 16-bit header (harus jumlah semua word = 0 mod 0x10000). Dilaporkan OK/BAD per FV yang ditemukan.
2. **FFS file checksum** (di dalam tiap FV) — untuk file dengan header klasik (24-byte, non-extended):
   - Header checksum (`IntegrityCheck.Header`) — divalidasi selalu.
   - Data checksum (`IntegrityCheck.File`) — **hanya berlaku kalau bit `FFS_ATTRIB_CHECKSUM` (0x40) di-set** pada file itu. Kalau ga di-set, byte itu memang FIXED di 0xAA sesuai spec (bukan checksum asli) — tool ga akan "fix" bagian ini, karena memang ga perlu.
3. **`--fix-uefi-checksums`**: setelah patch DMI, cari FFS file mana yang *data region*-nya overlap dengan range DMI yang baru saja di-swap, lalu recompute `IntegrityCheck.File` byte-nya kalau file itu emang butuh (attrib checksum di-set).

**Hasil test nyata** (pakai `BIOS.cap` asli hasil extract `e8cn39ww.exe` via innoextract):
- Tool berhasil temukan 6 real Firmware Volume di dalam capsule, semua header checksum OK.
- Region DMI (0x1000-0x2FFF) di file `.cap` ini ternyata **belum overlap FFS file manapun** (masih di area sebelum FV pertama / padding) — jadi `--fix-uefi-checksums` correctly report "no fix needed" alih-alih ngasal klaim sukses.
- Ini artinya: **tool jujur soal kondisi file lo**, bukan asal bilang "checksum fixed" — kalau ternyata patch DMI lo di offset yang beda (misal setelah full chip dump, bukan capsule installer), baru overlap ke FFS file beneran dan fixup jalan.

**Kenapa bisa gitu (raw chip dump vs `.cap` installer file):**
`BIOS.cap` yang diextract dari installer EXE itu bentuknya sedikit beda dengan full raw dump dari chip fisik (yang biasanya ada header/region tambahan sebelum FV utama, tergantung tools capture-nya). Makanya **selalu jalankan `--verify-uefi` dulu di dump chip ASLI lo** (bukan cuma di file installer capsule) buat lihat FV/FFS beneran yang overlap DMI range lo — offset absolut bisa geser.

## Source riset
- https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/102197-copy-dmi-info-easily-with-hex-editing-software-and-macro-script (guide asli, Tiny Hexer + macro, offset 0x520000-0x5207FF contoh generic)
- https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/85892-ideapad-3-14ada05-15ada05-17ada05-lenovo-v14-ada-v15-ada-bios (thread khusus ADA-series, sumber offset 0x1000-0x2FFF default di tool ini)
- UEFI Platform Initialization (PI) Specification, Volume 3 (Firmware Volume / FFS structures) — dipakai sebagai basis implementasi parser, bukan forum post
