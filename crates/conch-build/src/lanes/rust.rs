//! Rust lane: builds a CLI to wasip2 via cargo. Stub pending the compile spike.
//!
//! Per ADR #26 the Rust lane is "lowest friction *when it compiles*", but every
//! candidate still needs a spike (deps may assume threads/mmap/etc). Wire this
//! up alongside a concrete candidate (e.g. a uutils/jaq/ripgrep build).

use anyhow::{Result, bail};

use crate::manifest::Manifest;

pub fn build(manifest: &Manifest) -> Result<()> {
    bail!(
        "the Rust lane is not implemented yet (manifest '{}' selects lang=rust). \
         It needs a concrete compile spike — see ADR #26 / issue #51.",
        manifest.name
    )
}
