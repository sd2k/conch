//! C/C++ lane: builds a CLI to wasip2 via wasi-sdk's `wasm32-wasip2-clang`.
//! Stub pending the wasi-sdk lane + jq/curl spikes (#52, #79).
//!
//! Implementation note for when this lands: crib eryx's pattern — pin
//! `wasi-sdk-NN`, install on-demand via an `ensure-wasi-sdk` mise task, and keep
//! it OUT of PATH/`[tools]` (wasm-clang on PATH breaks host proc-macro builds).

use anyhow::{Result, bail};

use crate::manifest::Manifest;

pub fn build(manifest: &Manifest) -> Result<()> {
    bail!(
        "the C lane is not implemented yet (manifest '{}' selects lang=c). \
         It depends on the wasi-sdk lane + compile spike — see issues #52 (jq) and #79 (curl).",
        manifest.name
    )
}
