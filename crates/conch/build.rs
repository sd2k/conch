//! Build script: stages the embedded coreutils component for the
//! `embedded-coreutils` feature.
//!
//! `include_bytes!` requires the target file to exist at compile time, which
//! would make a plain `cargo build --features embedded-coreutils` fail when the
//! component hasn't been built yet. To stay graceful, we always write
//! `$OUT_DIR/coreutils.cwasm` — the real component if it's present under
//! `scratch/coreutils-component/`, otherwise an empty file. The runtime
//! auto-registration skips empty bytes, so an absent component just means
//! coreutils isn't available (not a compile error). Build it with
//! `mise run build-cli -- coreutils`.

use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR not set by cargo")?);
    let dest = out_dir.join("coreutils.cwasm");

    // Only relevant under the feature, but writing the placeholder unconditionally
    // is harmless and keeps include_bytes! happy if features change.
    // Repo root is two levels up from this crate (crates/conch).
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest_dir
        .join("../../scratch/coreutils-component/coreutils.cwasm")
        .canonicalize()
        .ok();

    match src {
        Some(path) if path.exists() => {
            std::fs::copy(&path, &dest)?;
            println!("cargo:rerun-if-changed={}", path.display());
        }
        _ => {
            std::fs::write(&dest, [])?;
        }
    }
    // Rerun if the component (re)appears.
    println!("cargo:rerun-if-changed=../../scratch/coreutils-component/coreutils.cwasm");
    Ok(())
}
