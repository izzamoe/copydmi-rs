# Flashing with CH341A and flashrom (Linux)

[← Back to README](../README.md)

This procedure was verified end-to-end on a Lenovo V15-ADA (`82C7A00RVN`) with a Winbond W25Q64.W 8 MiB chip: `flashrom` write verification passed and a separate readback matched the intended image SHA-256.

## 1. Install flashrom

On Arch/CachyOS, flashrom is normally in the official repositories; AUR/yay is not needed:

```bash
sudo pacman -S flashrom
```

## 2. Check the programmer

```bash
lsusb | grep -i "1a86:5512"
```

Expected text includes `QinHeng Electronics CH341 in EPP/MEM/I2C mode`.

## 3. Connect safely

Remove the BIOS chip from the motherboard, then attach the SOIC-8 clip/programmer. Do not program an in-circuit motherboard unless you have verified that its rails cannot conflict with the programmer.

## 4. Detect the chip

```bash
sudo flashrom -p ch341a_spi
```

Example:

```text
Found Winbond flash chip "W25Q64.W" (8192 kB, SPI) on ch341a_spi.
```

The detected capacity must match the generated output. `8192 kB` equals 8,388,608 bytes / 8 MiB. If flashrom reports `No EEPROM/flash device found`, reseat the clip; do not write.

## 5. Back up and prove read reliability

```bash
sudo flashrom -p ch341a_spi -r backup_chip_original_$(date +%Y%m%d).bin
sudo flashrom -p ch341a_spi -r read2.bin
sudo flashrom -p ch341a_spi -r read3.bin
sha256sum backup_chip_original_*.bin read2.bin read3.bin
```

All hashes must be identical. Different hashes mean the clip/programmer connection is unreliable.

## 6. Verify the output before writing

```bash
sha256sum bios_flash_ready.bin
```

Record this hash for the post-write readback comparison.

## 7. Write and verify

```bash
sudo flashrom -p ch341a_spi -w bios_flash_ready.bin
```

Do not move the clip during erase/write/verify. A successful operation ends with:

```text
Verifying flash... VERIFIED.
```

## 8. Independently read back the chip

```bash
sudo flashrom -p ch341a_spi -r verify_final.bin
sha256sum verify_final.bin bios_flash_ready.bin
```

The two hashes must match exactly. Only then reinstall the chip, observing pin-1 orientation.

## 9. First boot

Enter BIOS setup (often `F2` on Lenovo), choose **Load Setup Defaults** / **Load Optimized Defaults**, save, and reboot. Confirm the serial number and machine type when the firmware information page exposes them.

If an old Windows installation repeatedly enters Automatic Repair, a clean OS installation may be more reliable than attempting to repair the old installation.
