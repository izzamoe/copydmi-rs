# Research and implementation references

[← Back to README](../README.md)

## Lenovo DMI / LENV (`--show-dmi`)

- [github.com: LenovoDMIDecryptor](https://github.com/Shmurkio/LenovoDMIDecryptor) — primary reference for Lenovo `LENV` storage, XOR decrypt/encrypt, redundant blocks, entries, and the related `LDBG` change log. `copydmi` implements its own Rust decoder; it does not invoke that Windows binary.
- [winraid: lenovo-dmi-decryption-tool](https://winraid.level1techs.com/t/lenovo-dmi-decryption-tool/98137) — community thread by the decryptor author.
- [badcaps.net: 3286884-lenovo-dmi-decrypter](https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/3286884-lenovo-dmi-decrypter) — independent community corroboration of Lenovo DMI decrypt/extract/transfer use cases.
- [badcaps.net: 102197-copy-dmi-info-easily-with-hex-editing-software-and-macro-script](https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/102197-copy-dmi-info-easily-with-hex-editing-software-and-macro-script) — original manual CopyDMI guide; its `0x520000-0x5207FF` range is generic, not a universal default.
- [badcaps.net: 85892-ideapad-3-14ada05-15ada05-17ada05-lenovo-v14-ada-v15-ada-bios](https://www.badcaps.net/forum/troubleshooting-hardware-devices-and-electronics-theory/troubleshooting-laptops-tablets-and-mobile-devices/bios-requests-only/85892-ideapad-3-14ada05-15ada05-17ada05-lenovo-v14-ada-v15-ada-bios) — ADA-series community thread used during initial `0x1000-0x2FFF` research.
- [winraid: problem-bad-lenovo-legion-5-pro-bios-flash](https://winraid.level1techs.com/t/problem-bad-lenovo-legion-5-pro-bios-flash/39904) — independent evidence of related Lenovo InsydeH2O `LDBG` / `LENV` machine-specific regions.

## Firmware container and UEFI structures

- [github.com: extractor.c](https://github.com/LongSoft/InsydeImageExtractor/blob/master/extractor.c) — Insyde image extraction reference. The 82C7 wrapper is independently byte-validated because it differs from layouts handled by that extractor.
- [uefi.org: UEFI_PI_Spec_Final_Draft_1.9.pdf](https://uefi.org/sites/default/files/resources/UEFI_PI_Spec_Final_Draft_1.9.pdf) — UEFI Platform Initialization specification, used for FV/FFS parsing and checksum handling.
- Independent byte-level validation of official `e8cn39ww.exe` and `e8cn41ww.exe`: clean firmware starts at `0x318` and is exactly `0x800000` bytes. The recorded clean firmware SHA-256 is `28049cb8efd57c04a8f879f2bdc66bd43b4ceae2a8c5f231369f3baf80f19c02`.

## Hardware flashing

- [flashrom.org: ch341ab.html](https://flashrom.org/supported_hw/supported_prog/ch341ab.html) — upstream CH341A/B programmer documentation.
- [flashrom.org: classic_cli_manpage.html](https://flashrom.org/classic_cli_manpage.html) — upstream flashrom CLI documentation, including `ch341a_spi`, read, write, and verification operations.
- [winraid: guide-how-to-use-a-ch341a-spi-programmer-flasher](https://winraid.level1techs.com/t/guide-how-to-use-a-ch341a-spi-programmer-flasher/33041) — additional community CH341A/SPI guide.
