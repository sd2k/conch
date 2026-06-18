//! C/C++ lane: compiles a CLI to a **wasip2 component** via wasi-sdk's
//! `wasm32-wasip2` clang.
//!
//! Unlike the Go lane there is no separate componentization step: wasi-sdk's
//! `wasm32-wasip2` target links via `wasm-component-ld`, so the linked
//! executable is already a component (it imports `wasi:*@0.2.x` and exports
//! `wasi:cli/run`).
//!
//! Two build systems, selected by `[build] system`:
//! - `single` — one `clang` invocation over `[build] sources` (amalgamation
//!   style, e.g. sqlite3).
//! - `cmake` — configure + build a CMake project with the wasi-sdk toolchain
//!   file, then copy `[build] artifact` out of the build dir (e.g. curl).
//!
//! Toolchain input (not per-CLI, so sourced from the environment):
//! - `WASI_SDK_PATH` — root of the wasi-sdk install. The mise `ensure-wasi-sdk`
//!   task installs it on demand and the build tasks export it. It is
//!   deliberately kept OUT of `[tools]`/PATH because a wasm clang on PATH breaks
//!   host proc-macro builds (eryx's hard-won note).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{compile_cwasm, path_str, run};
use crate::manifest::{CBuildSystem, Manifest};

pub fn build(manifest: &Manifest) -> Result<()> {
    let repo_root = std::env::current_dir().context("getting current dir")?;

    // Resolve the wasi-sdk (off PATH on purpose — see module docs).
    let sdk = match std::env::var_os("WASI_SDK_PATH") {
        Some(p) => PathBuf::from(p),
        None => bail!(
            "WASI_SDK_PATH not set — install the SDK with `mise run ensure-wasi-sdk` \
             (it stays off PATH; the build-cli/demo tasks export it)"
        ),
    };
    let clang = sdk.join("bin/clang");
    if !clang.exists() {
        bail!(
            "clang not found at {} — incomplete wasi-sdk install?",
            clang.display()
        );
    }

    let source_dir = repo_root.join(&manifest.source.dir);
    if !source_dir.exists() {
        bail!(
            "source dir {} not found — fetch {} @ {} there (e.g. `mise run fetch-curl`)",
            source_dir.display(),
            manifest.source.repo,
            manifest.source.git_ref
        );
    }
    let output_dir = repo_root.join(&manifest.output.dir);
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("creating output dir {}", output_dir.display()))?;

    // Optional patch hook (mirrors the Go lane), applied to the source tree
    // before compiling — e.g. working around wasi-libc gaps.
    if let Some(patch) = &manifest.build.vendor_patch {
        eprintln!("=== [{}] patch source tree ===", manifest.name);
        let patch_abs = repo_root.join(patch);
        if !patch_abs.exists() {
            bail!("vendor_patch script {} not found", patch_abs.display());
        }
        let patch_s = path_str(&patch_abs)?;
        run("bash", &[&patch_s], &source_dir, &[])?;
    }

    let component = output_dir.join("component.wasm");
    match manifest.build.system {
        CBuildSystem::Single => {
            build_single(manifest, &clang, &source_dir, &component)?;
        }
        CBuildSystem::Cmake => {
            build_cmake(manifest, &sdk, &source_dir, &output_dir, &component)?;
        }
    }

    compile_cwasm(manifest, &component, &output_dir, &repo_root)?;

    eprintln!("=== [{}] done → {} ===", manifest.name, component.display());
    Ok(())
}

/// `single` mode: one `clang --target=wasm32-wasip2` invocation that links a
/// component directly.
fn build_single(
    manifest: &Manifest,
    clang: &Path,
    source_dir: &Path,
    component: &Path,
) -> Result<()> {
    if manifest.build.sources.is_empty() {
        bail!(
            "the C lane (system=single) needs `[build] sources = [...]` for '{}'",
            manifest.name
        );
    }
    eprintln!(
        "=== [{}] compile + link to wasip2 component ===",
        manifest.name
    );
    let clang_s = path_str(clang)?;
    let component_s = path_str(component)?;

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
    args.push(component_s);
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&clang_s, &args_ref, source_dir, &[])
}

/// `cmake` mode: configure + build a CMake project with the wasi-sdk wasip2
/// toolchain file, then copy the built artifact to `component.wasm`.
fn build_cmake(
    manifest: &Manifest,
    sdk: &Path,
    source_dir: &Path,
    output_dir: &Path,
    component: &Path,
) -> Result<()> {
    let artifact_rel = manifest.build.artifact.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "the C lane (system=cmake) needs `[build] artifact = \"...\"` for '{}'",
            manifest.name
        )
    })?;

    let toolchain = sdk.join("share/cmake/wasi-sdk-p2.cmake");
    if !toolchain.exists() {
        bail!(
            "wasip2 CMake toolchain file not found at {}",
            toolchain.display()
        );
    }
    let build_dir = output_dir.join("build");

    let toolchain_arg = format!("-DCMAKE_TOOLCHAIN_FILE={}", path_str(&toolchain)?);
    let cflags_arg = format!("-DCMAKE_C_FLAGS={}", manifest.build.cflags.join(" "));
    let linker_arg = format!(
        "-DCMAKE_EXE_LINKER_FLAGS={}",
        manifest.build.link_flags.join(" ")
    );
    let build_dir_s = path_str(&build_dir)?;
    let source_dir_s = path_str(source_dir)?;

    eprintln!("=== [{}] cmake configure ===", manifest.name);
    let mut cfg: Vec<String> = vec![
        "-B".into(),
        build_dir_s.clone(),
        "-S".into(),
        source_dir_s,
        "-G".into(),
        "Ninja".into(),
        toolchain_arg,
        cflags_arg,
        linker_arg,
    ];
    cfg.extend(manifest.build.cmake_flags.iter().cloned());
    let cfg_ref: Vec<&str> = cfg.iter().map(String::as_str).collect();
    run("cmake", &cfg_ref, source_dir, &[])?;

    eprintln!("=== [{}] cmake build ===", manifest.name);
    run("cmake", &["--build", &build_dir_s], source_dir, &[])?;

    let artifact = build_dir.join(artifact_rel);
    if !artifact.exists() {
        bail!(
            "expected CMake artifact {} not found after build",
            artifact.display()
        );
    }
    std::fs::copy(&artifact, component)
        .with_context(|| format!("copying {} → {}", artifact.display(), component.display()))?;
    eprintln!("  {} → {}", artifact.display(), component.display());
    Ok(())
}
