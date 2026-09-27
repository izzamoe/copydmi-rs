//! copydmi - Rust CLI to transplant DMI/SMBIOS info block between Lenovo BIOS dumps
//! Use case: Lenovo V15-ADA / V14-ADA / IdeaPad 3 xxADA05 (E8CN series, chip 82C7 etc)
//! Replicates the community "CopyDMI" macro workflow (Tiny Hexer) from badcaps.net,
//! but as a scriptable/auditable CLI instead of a black-box .mps macro file.
//!
//! DEFAULT OFFSETS are for the ADA-series (82C7 / E8CN) BIOS: DMI region 0x1000-0x2FFF
//! RELATIVE TO THE REAL FIRMWARE IMAGE START (per badcaps.net thread "IdeaPad 3
//! 14ADA5/15ADA05/17ADA05 Lenovo V14-ADA/V15-ADA Bios"). See the `image` module for
//! the clean-header stripping that makes those offsets line up.
//! ALWAYS double check offsets against your own verified source before trusting blindly.
//!
//! USAGE:
//!   copydmi --old oldbios.bin --new newbios.bin --out patched.bin
//!   copydmi --old oldbios.bin --new newbios.bin --out patched.bin --start 0x1000 --end 0x2FFF
//!   copydmi --old oldbios.bin --new newbios.bin --out patched.bin --dry-run
//!   copydmi --old oldbios.bin --new newbios.exe --out patched.bin --chip-size 0x1000000
//!   copydmi --old oldbios.bin --new newbios.bin --out patched.bin --no-trim
//!
//! SAFETY:
//!   - Never overwrites newbios.bin/oldbios.bin in place; always writes to --out.
//!   - Refuses to run if --out already exists, unless --force is passed.
//!   - Verifies newbios.bin size before and after patch matches (BIOS image size must not change).
//!   - Prints a hex diff summary of the patched region before writing, unless --quiet.
//!   - Header-stripping removes bytes from the FRONT (the wrapper), then trims any
//!     remaining tail metadata down to --chip-size. Both steps are clearly logged
//!     with exact byte counts so the operation is fully auditable before you flash.
//!
//! MODULES:
//!   - `cli`        argument parsing, validation, help text
//!   - `loader`     reading raw dumps / extracting firmware from installer .exe files
//!   - `image`      header stripping, chip-size trimming, range size checks
//!   - `transplant` DMI range diff/copy and hex preview
//!   - `uefi`       FV/FFS scanning, checksum reporting and repair
//!   - `lenv`       Lenovo LENV/DMI block decryption, display and verification
//!   - `app`        the end-to-end pipeline tying the above together

mod app;
mod cli;
mod image;
mod lenv;
mod loader;
mod transplant;
mod uefi;

use std::env;
use std::process::ExitCode;

use cli::Command;

fn main() -> ExitCode {
    let result = match cli::parse_args(env::args().skip(1)) {
        Ok(Command::Help) => {
            cli::print_usage();
            return ExitCode::SUCCESS;
        }
        Ok(Command::Run(args)) => app::run(&args),
        Err(e) => Err(e),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("run with -h for usage");
            ExitCode::FAILURE
        }
    }
}
