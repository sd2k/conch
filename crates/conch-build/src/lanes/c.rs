//! C/C++ lane: compiles a CLI to a **wasip2 component** via wasi-sdk's
//! `wasm32-wasip2` clang.
//!
//! Unlike the Go lane there is no separate componentization step: wasi-sdk's
//! `wasm32-wasip2` target links via `wasm-component-ld`, so clang emits a
//! component directly (it imports `wasi:*@0.2.x` and exports `wasi:cli/run`).
//! The build is therefore just: optional patch → one `clang` invocation →
//! optional cwasm.
//!
//! Toolchain input (not per-CLI, so sourced from the environment):
//! - `WASI_SDK_PATH` — root of the wasi-sdk install. The mise `ensure-wasi-sdk`
//!   task installs it on demand and the `build-cli`/`demo-sqlite` tasks export
//!   it. It is deliberately kept OUT of `[tools]`/PATH because a wasm clang on
//!   PATH breaks host proc-macro builds (eryx's hard-won note).
//!
//! Per-CLI build config comes from the manifest's `[build]` table:
//! `sources` (the C files), `cflags` (defines/opt/feature toggles), and
//! `link_flags` (e.g. wasi-sdk emulation libs).

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use super::{compile_cwasm, path_str, run};
use crate::manifest::Manifest;

pub fn build(manifest: &Manifest) -> Result<()> {
    let repo_root = std::env::current_dir().context("getting current dir")?;

    // Resolve the wasi-sdk (off PATH on purpose — see module docs).
    let sdk = match std::env::var_os("WASI_SDK_PATH") {
        Some(p) => PathBuf::from(p),
        None => bail!(
            "WASI_SDK_PATH not set — install the SDK with `mise run ensure-wasi-sdk` \
             (it stays off PATH; the build-cli/demo-sqlite tasks export it)"
        ),
    };
    let clang = sdk.join("bin/clang");
    if !clang.exists() {
        bail!(
            "clang not found at {} — incomplete wasi-sdk install?",
            clang.display()
        );
    }

    if manifest.build.sources.is_empty() {
        bail!(
            "the C lane needs `[build] sources = [...]` in the manifest for '{}'",
            manifest.name
        );
    }

    let source_dir = repo_root.join(&manifest.source.dir);
    if !source_dir.exists() {
        bail!(
            "source dir {} not found — fetch {} @ {} there (e.g. `mise run fetch-sqlite`)",
            source_dir.display(),
            manifest.source.repo,
            manifest.source.git_ref
        );
    }
    let output_dir = repo_root.join(&manifest.output.dir);
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("creating output dir {}", output_dir.display()))?;

    // Optional patch hook (mirrors the Go lane), run before compiling.
    if let Some(patch) = &manifest.build.vendor_patch {
        eprintln!("=== [{}] patch source tree ===", manifest.name);
        let patch_abs = repo_root.join(patch);
        if !patch_abs.exists() {
            bail!("vendor_patch script {} not found", patch_abs.display());
        }
        let patch_s = path_str(&patch_abs)?;
        run("bash", &[&patch_s], &source_dir, &[])?;
    }

    // Single clang invocation: clang emits a component directly for wasip2.
    eprintln!(
        "=== [{}] compile + link to wasip2 component ===",
        manifest.name
    );
    let clang_s = path_str(&clang)?;
    let component = output_dir.join("component.wasm");
    let component_s = path_str(&component)?;

    let mut args: Vec<String> = vec!["--target=wasm32-wasip2".to_string()];
    args.extend(manifest.build.cflags.iter().cloned());
    args.extend(
        manifest
            .build
            .sources
            .iter()
            .map(|s| s.to_string_lossy().into_owned()),
    );
    args.extend(manifest.build.link_flags.iter().cloned());
    args.push("-o".to_string());
    args.push(component_s.clone());
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&clang_s, &args_ref, &source_dir, &[])?;

    compile_cwasm(manifest, &component, &output_dir, &repo_root)?;

    eprintln!("=== [{}] done → {} ===", manifest.name, component.display());
    Ok(())
}
