# copydmi (Rust) — DMI/SMBIOS Transplant + Clean-BIOS Tool untuk Lenovo

[![CI](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

CLI Rust yang meng-otomatisasi proses "CopyDMI" ala komunitas badcaps.net (biasanya dikerjakan manual pakai Tiny Hexer + macro `.mps`), **plus** "clean BIOS" (strip capsule/wrapper header) — hasil akhirnya file `.bin` **siap flash langsung** ke chip pakai CH341A, bukan cuma capsule mentah.

## Kenapa dibikin
Attachment `CopyDMI.zip`/`CopyDMI.mps` di forum itu file macro proprietary buat software Windows (Tiny Hexer), sering ke-lock di balik requirement post-count/premium di forum. Tool ini reimplementasi logic yang sama (copy byte range tertentu dari file A ke file B) sebagai CLI portable, auditable, cross-platform — ga perlu Windows/Tiny Hexer, **dan** otomatis strip header capsule yang biasanya dikerjakan manual pakai HxD.

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

## Pakai (cara paling simpel — semua otomatis)
```bash
./copydmi --old dump_asli_lo.bin --new e8cn39ww.exe --out bios_siap_flash.bin --fix-uefi-checksums
```

`--new` (dan `--old`) bisa langsung terima:
- File `.exe` installer resmi Lenovo — otomatis diextract in-memory (pure Rust, crate `inno`, ga butuh innoextract/7z/Docker)
- File raw dump (`.bin`/`.cap`/`.fd`/`.rom`)

Untuk **model ADA-series (V15-ADA/82C7, E8CN family)**, tool ini otomatis (default ON, ga perlu flag tambahan):
1. **Strip 792-byte capsule/wrapper header** dari depan file — ini step "clean BIOS" yang biasanya dikerjakan manual pakai HxD/hex editor
2. **Trim ke ukuran chip fisik 8MB (8,388,608 bytes)** dari belakang — buang sisa metadata installer/debug string
3. Transplant DMI (`0x1000-0x2FFF`, relatif ke firmware bersih setelah strip)
4. (opsional `--fix-uefi-checksums`) repair checksum FFS yang overlap DMI

Hasil akhir `--out` itu file **8MB persis, siap ditulis langsung ke chip** — bukan capsule + metadata installer.

## Riset di balik "clean BIOS" step (jangan skip baca ini sebelum flash)

**Masalah**: file `.cap`/`.fd` hasil extract installer Lenovo (`e8cn39ww.exe`, `e8cn41ww.exe`, dll) itu **8,950,768 bytes** — lebih besar dari chip fisik (Winbond W25Q64-class, 8MB = 8,388,608 bytes). Kalau langsung diflash apa adanya, hasilnya salah/corrupt.

**Struktur yang sudah diverifikasi byte-per-byte** (terhadap 2 versi BIOS berbeda: e8cn39ww.exe DAN e8cn41ww.exe — hasil identik di keduanya):

| Offset (raw file) | Panjang | Isi |
|---|---|---|
| `0x000` | 80 byte | `EFI_CAPSULE_HEADER` (GUID, HeaderSize=0x50, Flags, CapsuleImageSize) |
| `0x050` | 72 byte | Outer Firmware Volume header (wrapper, membungkus seluruh isi sebagai 1 FFS file besar) |
| `0x098` | 640 byte | FFS file header + signature/crypto blob |
| **`0x318`** | **8,388,608 byte (8MB persis)** | **FIRMWARE IMAGE ASLI — ini yang harus di-flash** |
| `0x800318` | 562,152 byte sisa | Debug strings, PDB path, metadata installer (buangan) |

**Bukti yang mengonfirmasi (bukan tebakan):**
1. Base offset `0x318` **identik** di 2 versi BIOS berbeda (e8cn39ww & e8cn41ww)
2. Semua Firmware Volume valid (`_FVH` signature + header checksum OK) jatuh di alamat **bulat rapi** kalau dihitung relatif ke base `0x318`: `0x310000`, `0x360000`, `0x390000`, `0x3A0000`, `0x730000` — dan rantainya **kontigu sempurna, berakhir TEPAT di 0x800000** (8MB)
3. String SMBIOS/DMI vendor `"LENV\0"` persis jatuh di **relative offset 0x1000** dan **0x2000** — pas di awal & tengah rentang DMI resmi komunitas (`0x1000-0x2FFF`)
4. Forum winraid.level1techs.com soal Lenovo Legion 5 Pro (BIOS Insyde sekeluarga) mengonfirmasi pola region bernama `LDBG`/`LENV*` sebagai area DMI/machine-specific data

**Bug yang ditemukan & sudah diperbaiki**: versi tool sebelumnya transplant DMI di offset `0x1000` dari **raw file** (area kosong `FF FF FF...`) — bukan `0x1000` relatif ke firmware bersih (yaitu raw offset `0x1318`, isinya `LENV`). Sekarang sudah benar: strip header dulu, baru transplant di offset relatif yang tepat.

## Opsi CLI

```
--start <HEX|DEC>       Start offset DMI, RELATIF ke firmware setelah header-strip (default 0x1000)
--end <HEX|DEC>         End offset DMI, inclusive (default 0x2FFF)
--header-size <HEX|DEC> Bytes yang di-strip dari DEPAN --new sebelum apapun — step "clean BIOS"
                        (default 0x318 / 792 bytes, khusus ADA-series/82C7/E8CN)
--old-header-size <N>   Bytes yang di-strip dari DEPAN --old (default: sama dengan --header-size).
                        Pakai ini kalau --old dan --new punya wrapper BEDA — contoh paling umum:
                        --old itu dump chip mentah yang SUDAH bersih (mulai langsung dari "LDBG",
                        tanpa capsule wrapper) sedangkan --new masih installer .exe yang perlu
                        di-strip 792 byte. Tanpa flag ini, --old ikut ke-strip lagi dan DMI offset-nya
                        geser jadi salah (hasil garbage, bukan error yang kelihatan).
--chip-size <HEX|DEC>   Ukuran target chip, trim dari BELAKANG setelah header-strip
                        (default 0x800000 / 8MB, khusus ADA-series/82C7/E8CN)
--no-trim               Matikan strip header + trim chip-size sepenuhnya (pakai file apa adanya)
--force                 Timpa file --out kalau udah ada
--dry-run               Cek dulu tanpa nulis file (liat berapa byte yang beda, preview hex)
--quiet                 Skip preview hex
--verify-uefi           Scan FV/FFS, laporkan validitas checksum (real parsing, bukan tebakan)
--fix-uefi-checksums    Setelah patch, perbaiki checksum FFS yang overlap DMI (implies --verify-uefi)
--show-dmi              Decode & tampilkan isi blok DMI (LENV) --old/--new/hasil patch secara
                        manusiawi (Serial Number, Machine Type, UUID, dll) — dan kalau dipakai
                        bareng transplant sungguhan (bukan --dry-run), kasih verdict eksplisit
                        MATCH/MISMATCH per blok dibanding --old. Ini cara TERCEPAT buat verifikasi
                        transplant berhasil, tanpa perlu script decode manual/tool eksternal.
```

**Model lain / offset beda**: override manual, contoh:
```bash
./copydmi --old old.bin --new new.bin --out out.bin --header-size 0 --start 0x520000 --end 0x5207FF --no-trim
```

## Alur pemakaian lengkap (kasus Izzam - 82C7)
1. Dump chip BIOS lo yang sekarang pakai CH341A (walau corrupt) → simpan sebagai `dump_asli.bin`
2. Download BIOS resmi dari Lenovo (`e8cn39ww.exe` / `e8cn41ww.exe`) — **ga perlu extract manual**, tinggal pakai langsung
3. Jalankan copydmi, kasih EXE-nya langsung sebagai `--new`:
   ```bash
   ./copydmi --old dump_asli.bin --new e8cn39ww.exe --out bios_final.bin --fix-uefi-checksums --show-dmi
   ```
   Kalau `dump_asli.bin` itu dump chip mentah yang **sudah bersih** (langsung mulai dari `LDBG`, bukan hasil extract installer), tambahkan `--old-header-size 0` — kalau tidak, DMI-nya ikut ke-strip salah offset dan hasil transplant jadi garbage tanpa error yang kelihatan.
4. Cek output log — pastikan "bytes differing" masuk akal (ga 0, ga 100%), size `--out` = 8,388,608 bytes, dan verdict `--show-dmi` bilang **MATCH** untuk kedua blok (0x1000 dan 0x2000). Kalau MISMATCH, tool bakal print warning `#### DO NOT FLASH ####` — jangan lanjut ke tahap flash kalau ini muncul.
5. Flash `bios_final.bin` ke chip pakai `flashrom` via CH341A — lihat section di bawah untuk tutorial lengkap. File ini **sudah siap tulis langsung**, ga perlu proses tambahan di HxD.

## Tutorial flash ke hardware (CH341A + flashrom di Linux)

Diverifikasi end-to-end di kasus nyata (Lenovo V15-ADA 82C7A00RVN, S/N PC1VSPVC) — chip Winbond W25Q64.W 8MB, hasil `flashrom -w` + readback independen match SHA-256.

**1. Install flashrom**
Cek dulu apakah ada di repo resmi distro lo (Arch/CachyOS: `pacman -Si flashrom`) sebelum pakai AUR — biasanya udah tersedia di repo utama, ga perlu `yay`.
```bash
sudo pacman -S flashrom   # Arch/CachyOS, atau apt/dnf sesuai distro
```

**2. Sambungkan CH341A, pastikan terdeteksi OS**
```bash
lsusb | grep -i "1a86:5512"
```
Harus muncul baris `QinHeng Electronics CH341 in EPP/MEM/I2C mode`.

**3. Lepas chip BIOS dari motherboard**, clip pakai SOIC-8 test clip ke CH341A (jangan solder langsung kalau bisa clip; chip harus benar-benar lepas dari board, bukan in-circuit, supaya ga ada tegangan bentrok).

**4. Deteksi chip (WAJIB sebelum operasi apapun)**
```bash
sudo flashrom -p ch341a_spi
```
Kalau muncul "No EEPROM/flash device found" — klip belum nempel pas, coba ulang/perbaiki posisi klip. Kalau berhasil, muncul nama chip + ukurannya (contoh: `Found Winbond flash chip "W25Q64.W" (8192 kB, SPI)`) — cocokkan ukuran ini dengan ukuran `--out` (8,388,608 bytes = 8192 kB).

**5. Backup isi chip sekarang — SELALU, tanpa kecuali, walau chip-nya sudah bermasalah**
```bash
sudo flashrom -p ch341a_spi -r backup_chip_asli_$(date +%Y%m%d).bin
```
Verifikasi **reliabilitas bacaan** sebelum lanjut — dump 2-3x lagi tanpa gerakin klip, cocokkan SHA-256-nya:
```bash
sudo flashrom -p ch341a_spi -r read2.bin
sha256sum backup_chip_asli_*.bin read2.bin
```
Kalau hash beda antar dump → klip belum stabil, JANGAN lanjut ke write, benerin dulu kontaknya. Kalau hash sama semua → bacaan reliable, lanjut.

**6. Verifikasi checksum file yang mau di-flash** sebelum ditulis (pastikan transfer/copy file ga korup):
```bash
sha256sum bios_final.bin   # cocokkan dengan checksum yang kamu catat pas generate filenya
```

**7. Write (flashrom otomatis read-before-write + verify di akhir)**
```bash
sudo flashrom -p ch341a_spi -w bios_final.bin
```
Tunggu sampai selesai tanpa gerakin klip (bisa beberapa menit — proses ini erase+write+verify seluruh 8MB). Output akhir harus `Verifying flash... VERIFIED.` — kalau ada error di tengah, JANGAN cabut klip, retry write lagi (chip belum tentu rusak, banyak kasus itu cuma retry biasa perlu diulang; backup langkah 5 jadi fallback kalau semua retry gagal).

**8. Verifikasi independen tambahan** (jangan cuma percaya kata "VERIFIED" — baca ulang dan bandingkan manual):
```bash
sudo flashrom -p ch341a_spi -r verify_final.bin
sha256sum verify_final.bin   # harus match persis checksum bios_final.bin
```

**9. Pasang chip balik** ke motherboard (perhatikan orientasi/pin 1), rakit ulang, nyalakan.

**10. Setelah nyala**: masuk BIOS setup (F2 di kebanyakan Lenovo), pilih **Load Setup Defaults / Load Optimized Defaults** dulu sebelum boot ke OS — settingan CMOS lama biasanya ga sinkron sama firmware yang baru ditulis. Kalau S/N & Machine Type sudah benar di layar System Information, transplant DMI berhasil.

**Kalau Windows lama sempat rusak/"Preparing automatic repair" gara-gara histori BIOS bermasalah** (DMI kosong/reset sebelum di-transplant): install ulang Windows bersih (custom install, hapus partisi lama) biasanya lebih cepat & pasti daripada nunggu auto-repair yang kemungkinan gagal terus. Kalau Windows dulu OEM-licensed, digital license biasanya otomatis balik aktif begitu online, karena hardware ID (S/N/UUID) sekarang sudah benar lagi.

## PENTING — Batasan tool ini
- Cuma swap byte range DMI + (opsional) fix checksum FFS-level yang overlap range itu. TIDAK menangani extended FFS header, GUIDed/compressed section internals, Insyde vendor signing, atau secure-boot key.
- Default `--header-size`/`--chip-size`/`--start`/`--end` itu terverifikasi khusus untuk **ADA-series/82C7/E8CN family** (V14-ADA, V15-ADA, IdeaPad 3 14/15/17ADA05) — model Lenovo lain kemungkinan besar beda wrapper/offset. Selalu cross-check kalau model lo bukan seri ini.
- Selalu simpan backup `dump_asli.bin` yang ORIGINAL (belum diapa-apain) terpisah, jangan overwrite.

## Fitur checksum UEFI (--verify-uefi / --fix-uefi-checksums)

Tool ini mem-parsing struktur **UEFI Firmware Volume (FV)** dan **Firmware File System (FFS)** sesuai spec resmi (UEFI PI Volume 3) — bukan asumsi/tebakan:

**Apa yang dicek:**
1. **FV header checksum** — scan seluruh file cari signature `_FVH`, validasi checksum 16-bit header (harus jumlah semua word = 0 mod 0x10000). Dilaporkan OK/BAD per FV yang ditemukan.
2. **FFS file checksum** (di dalam tiap FV) — untuk file dengan header klasik (24-byte, non-extended):
   - Header checksum (`IntegrityCheck.Header`) — divalidasi selalu.
   - Data checksum (`IntegrityCheck.File`) — **hanya berlaku kalau bit `FFS_ATTRIB_CHECKSUM` (0x40) di-set** pada file itu. Kalau ga di-set, byte itu memang FIXED di 0xAA sesuai spec (bukan checksum asli) — tool ga akan "fix" bagian ini, karena memang ga perlu.
3. **`--fix-uefi-checksums`**: setelah patch DMI, cari FFS file mana yang *data region*-nya overlap dengan range DMI yang baru saja di-swap, lalu recompute `IntegrityCheck.File` byte-nya kalau file itu emang butuh (attrib checksum di-set).

**Hasil test nyata** (pakai `e8cn39ww.exe` asli, setelah fix header-strip):
- Tool menemukan 5 real Firmware Volume di firmware bersih 8MB, semua header checksum OK, dan FV terakhir berhasil parse 158 FFS file di dalamnya (sebelum fix header-strip, parsing FV terakhir terpotong karena base offset salah).
- Region DMI (`0x1000-0x2FFF` relatif) sekarang correctly overlap dengan data SMBIOS asli (`LENV` string terbaca di posisi yang tepat).

## Source riset
- https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/102197-copy-dmi-info-easily-with-hex-editing-software-and-macro-script (guide asli, Tiny Hexer + macro, offset 0x520000-0x5207FF contoh generic)
- https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/85892-ideapad-3-14ada05-15ada05-17ada05-lenovo-v14-ada-v15-ada-bios (thread khusus ADA-series, sumber offset 0x1000-0x2FFF)
- https://winraid.level1techs.com/t/problem-bad-lenovo-legion-5-pro-bios-flash/39904 (konfirmasi independen pola region LDBG/LENV* sebagai area DMI, BIOS Insyde sekeluarga)
- https://github.com/LongSoft/InsydeImageExtractor/blob/master/extractor.c (referensi cara kerja extractor resmi Insyde image, meski model 82C7 pakai wrapper berbeda dari yang ditangani tool ini)
- UEFI Platform Initialization (PI) Specification, Volume 3 (Firmware Volume / FFS structures) — dipakai sebagai basis implementasi parser, bukan forum post
- Verifikasi mandiri: analisis byte-level terhadap `e8cn39ww.exe` dan `e8cn41ww.exe` asli (SHA-256 firmware bersih hasil strip: `28049cb8efd57c04a8f879f2bdc66bd43b4ceae2a8c5f231369f3baf80f19c02`)
