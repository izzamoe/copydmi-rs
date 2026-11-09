# copydmi

[English](README.md) | **Bahasa Indonesia**

[![CI](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/izzamoe/copydmi-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

Mengembalikan serial number, UUID, dan model (DMI/SMBIOS) laptop Lenovo setelah chip BIOS terhapus atau diganti. `copydmi` mengambil dump chip asli plus BIOS resmi Lenovo, lalu menghasilkan image mentah siap flash pakai CH341A / `flashrom`.

> **Peringatan:** flash BIOS bisa membuat perangkat mati permanen. Simpan beberapa backup identik dari chip asli, cek hash SHA-256-nya, dan pastikan offset sesuai model Lenovo kamu sebelum menulis apa pun.

## Fungsi

- Membaca installer `.exe` resmi Lenovo langsung (diekstrak di memori, tanpa 7-Zip atau Docker) atau file mentah `.bin` / `.cap` / `.fd` / `.rom`.
- Membuang wrapper installer dan memotong ke ukuran chip fisik.
- Menyalin region DMI dari dump lama ke firmware baru.
- Men-decode blok DMI `LENV` Lenovo dan **menolak menulis** image yang tidak membawa DMI kamu dengan utuh.
- Opsional: memperbaiki checksum FFS UEFI yang overlap dengan range DMI.

**Dipakai untuk:** mesin Lenovo InsydeH2O dengan penyimpanan DMI `LENV`. Default tervalidasi untuk seri ADA (V14-ADA, V15-ADA, IdeaPad 3 14/15/17ADA05, keluarga BIOS E8CN).
**Jangan dipakai asal untuk:** keluarga Lenovo lain atau merek lain. Offset dan ukuran wrapper beda, lihat [Model lain](#model-lain).

## Mulai cepat

1. Install ([detail](#install)):

   ```bash
   curl -fsSL https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.sh | bash
   ```

2. Jalankan dengan dump asli dan installer resmi:

   ```bash
   copydmi --old dump_chip_asli.bin --new e8cn41ww.exe --out bios_siap_flash.bin \
     --fix-uefi-checksums --show-dmi
   ```

   Kalau dump berupa baca chip mentah yang sudah diawali `LDBG`, tambahkan `--old-header-size 0` ([alasan](#skenario)).

3. Cek hasil ([cara membaca](#membaca-hasil)). Yang diharapkan: `MATCH` di blok, exit code `0`, dan ukuran output persis seukuran chip (8.388.608 byte untuk seri ADA).

4. Flash dan verifikasi: [docs/flashing.md](docs/flashing.md) (bahasa Inggris).

Contoh output run yang berhasil (dump uji sintetis, bukan mesin asli):

```text
-- verdict --
  block @0x1000: MATCH — patched output carries the real old-bios DMI values unchanged
  block @0x2000: MATCH — patched output carries the real old-bios DMI values unchanged
  wrote patched bios: bios_siap_flash.bin (8388608 bytes)
```

Contoh run yang ditolak (dump lama tidak punya DMI sama sekali), exit code 1, tidak ada file ditulis:

```text
error: old bios has no usable LENV DMI block at 0x1000 or 0x2000 (wrong dump, wrong --old-header-size, ...
```

## Install

| Platform | Perintah |
|---|---|
| Linux / macOS | `curl -fsSL https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.sh \| bash` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.ps1 \| iex` |
| Manual | Unduh dari [Releases](https://github.com/izzamoe/copydmi-rs/releases) dan cek `SHA256SUMS.txt` |
| Dari source | `cargo build --release` (binary di `target/release/copydmi`) |

Script mengunduh rilis terbaru, memverifikasi SHA-256, lalu memasang ke PATH user (`~/.local/bin` atau `%LOCALAPPDATA%\Programs\copydmi`). Rust tidak perlu ada di komputer target. Buka terminal baru, lalu jalankan `copydmi --help`.

## Membaca hasil

Per blok DMI (`0x1000` dan `0x2000`):

| Verdict | Arti |
|---|---|
| `MATCH` | Blok lama dan hasil patch berisi entry identik. |
| `MISMATCH` | Hasil patch kehilangan atau mengubah DMI kamu. Error, tidak ada yang ditulis. **Jangan flash.** |
| `NO DATA` | Dump lama tidak punya data usable di blok ini, jadi tidak ada yang dibandingkan. Tidak pernah dihitung lulus. |

| Exit code | Arti |
|---|---|
| `0` | Output ditulis, setiap blok lama yang usable terverifikasi. |
| `1` | Ditolak atau gagal: tidak ada LENV usable di `--old`, verifikasi mismatch, output sudah ada tanpa `--force`, atau error ukuran/range. Untuk kegagalan DMI, file output tidak ditulis. |

Cek LENV selalu aktif (sejak v0.4.1) dan tidak bergantung pada `--show-dmi`; flag itu hanya menampilkan field hasil decode (serial, machine type, UUID). Pakai `--skip-dmi-check` hanya untuk model yang tidak menyimpan DMI sebagai LENV.

## Opsi

| Opsi | Default | Fungsi |
|---|---|---|
| `--old <FILE>` | wajib | Dump asli berisi DMI kamu (`.bin` atau `.exe` Lenovo). |
| `--new <FILE>` | wajib | Firmware resmi yang bersih (`.bin` atau `.exe` Lenovo). |
| `--out <FILE>` | wajib | Path output. File input tidak pernah ditimpa. |
| `--start <N>` / `--end <N>` | `0x1000` / `0x2FFF` | Range DMI inklusif, relatif ke image bersih. |
| `--header-size <N>` | `0x318` | Byte yang dibuang dari depan `--new`. |
| `--old-header-size <N>` | sama dengan `--header-size` | Byte yang dibuang dari depan `--old`. Pakai `0` untuk dump chip yang sudah bersih. |
| `--chip-size <N>` | `0x800000` (8 MiB) | Dipotong ke ukuran ini setelah strip. |
| `--no-trim` | off | Matikan strip header dan trim. |
| `--show-dmi` | off | Tampilkan field LENV hasil decode dan verdict per blok. |
| `--skip-dmi-check` | off | Matikan cek LENV fail-closed (hanya model non-LENV). |
| `--verify-uefi` | off | Laporkan status checksum FV/FFS. |
| `--fix-uefi-checksums` | off | Perbaiki checksum data FFS yang overlap range DMI (otomatis menyalakan `--verify-uefi`). |
| `--dry-run` | off | Validasi dan preview tanpa menulis. |
| `--quiet` | off | Sembunyikan preview hex. |
| `--force` | off | Timpa `--out` yang sudah ada. |

Angka boleh hex (`0x1000`) atau desimal.

## Skenario

**Seri ADA standar.** Default berlaku: buang `0x318` dari installer, potong ke 8 MiB, transplant `0x1000-0x2FFF`.

**Dump lama sudah bersih.** Baca chip mentah diawali `LDBG` dan tidak punya wrapper installer, sedangkan firmware baru masih berupa installer. Jangan strip kedua sisi:

```bash
copydmi --old dump_chip_bersih.bin --new e8cn41ww.exe --out bios_siap_flash.bin \
  --old-header-size 0 --show-dmi --fix-uefi-checksums
```

Tanpa `--old-header-size 0`, dump lama ikut ter-strip dua kali, offset bergeser, dan byte yang disalin salah.

### Model lain

Offset dan wrapper **tidak universal**. Override hanya setelah memvalidasi keluarga model kamu:

```bash
copydmi --old old.bin --new new.bin --out out.bin \
  --header-size 0 --start 0x520000 --end 0x5207FF --no-trim
```

## Alur yang disarankan

1. Baca chip fisik dengan CH341A dan simpan dump aslinya tanpa diubah.
2. Baca minimal dua kali lagi tanpa menggeser klip. Semua hash SHA-256 harus sama.
3. Unduh firmware resmi untuk keluarga mesin yang tepat.
4. Buat output dengan `--show-dmi` (tambahkan `--old-header-size 0` kalau perlu).
5. Pastikan `MATCH`, exit code `0`, dan ukuran output sesuai.
6. Tulis dengan `flashrom`, baca ulang, lalu bandingkan SHA-256 dengan file output.
7. Setelah dirakit, load BIOS setup defaults sebelum boot ke OS.

Prosedur hardware lengkap: [docs/flashing.md](docs/flashing.md) (bahasa Inggris).

## Batasan dan keamanan

- Hanya menyalin satu range byte dan bisa memperbaiki checksum data tingkat FFS yang relevan. Tidak memvalidasi semua signature vendor, section GUIDed/terkompresi, kunci secure-boot, extended FFS header, atau mekanisme integritas Insyde.
- Default offset, ukuran wrapper, dan ukuran chip hanya terverifikasi untuk seri ADA.
- `--show-dmi` hanya men-decode blok yang signature `LENV` dan struktur entry-nya lolos sanity check; tipe entry tak dikenal ditampilkan sebagai hex mentah.
- Jangan pernah menimpa satu-satunya backup chip asli.

## Dokumentasi lain (bahasa Inggris)

- [docs/flashing.md](docs/flashing.md): CH341A + flashrom di Linux, langkah demi langkah.
- [docs/technical.md](docs/technical.md): layout image seri ADA, dukungan checksum UEFI, verifikasi LENV.
- [docs/references.md](docs/references.md): sumber dan karya sebelumnya.
- [Releases](https://github.com/izzamoe/copydmi-rs/releases): binary dan changelog.

## Lisensi

MIT. Lihat [LICENSE](LICENSE).
