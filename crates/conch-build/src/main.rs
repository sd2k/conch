//! `conch-build` — a manifest-driven, multi-lane build driver for conch CLI
//! components.
//!
//! Reads a per-CLI manifest (`clis/<name>.toml`) and dispatches to the lane
//! selected by its `lang` field (rust | c | go). The goal (issue #51): adding a
//! CLI is a config change + spike, not a bespoke script.
//!
//! Usage:
//!
//! ```text
//! conch-build <name>          Build clis/<name>.toml (e.g. `conch-build gh`)
//! conch-build <path.toml>     Build a manifest at an explicit path
//! conch-build --list          List available manifests in clis/
//! conch-build --check         Validate all manifests without building (CI gate)
//! ```
//!
//! Run from the repo root (the mise `build-cli` task does this). Toolchains
//! (`wasm-tools`, `wasmtime`) come from PATH; the Go lane also needs the wasip3
//! Go fork (see `lanes::go`).

mod lanes;
mod manifest;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};

use crate::manifest::Manifest;

/// Directory holding per-CLI manifests, relative to the repo root.
const CLIS_DIR: &str = "clis";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("conch-build: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => {
            print_usage();
            Ok(())
        }
        Some("--list") => list_manifests(),
        Some("--check") => check_all_manifests(),
        Some(arg) => {
            let path = resolve_manifest_path(arg);
            let manifest = Manifest::load(&path)?;
            eprintln!(
                "conch-build: building '{}' ({}) via the {:?} lane — {} @ {}",
                manifest.name,
                manifest.description,
                manifest.lang,
                manifest.source.repo,
                manifest.source.git_ref,
            );
            lanes::build(&manifest)
        }
    }
}

/// Map a CLI argument to a manifest path: a `.toml` path is used as-is,
/// otherwise it's treated as a name under `clis/`.
fn resolve_manifest_path(arg: &str) -> PathBuf {
    if arg.ends_with(".toml") {
        PathBuf::from(arg)
    } else {
        PathBuf::from(CLIS_DIR).join(format!("{arg}.toml"))
    }
}

/// Collect the paths of all `*.toml` manifests in `clis/`, sorted.
fn manifest_paths() -> Result<Vec<PathBuf>> {
    let dir = PathBuf::from(CLIS_DIR);
    let entries = std::fs::read_dir(&dir)
        .with_context(|| format!("reading manifest dir {}", dir.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        bail!("no manifests found in {}", dir.display());
    }
    Ok(paths)
}

fn list_manifests() -> Result<()> {
    eprintln!("Available CLI manifests:");
    for path in manifest_paths()? {
        if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
            println!("  {name}");
        }
    }
    Ok(())
}

/// Validate every manifest in `clis/` without building: parse it, and check
/// any referenced `vendor_patch` script exists. Reproducible in CI (no
/// toolchains needed); guards against manifest bit-rot. Returns an error if any
/// manifest is invalid.
fn check_all_manifests() -> Result<()> {
    let paths = manifest_paths()?;
    let mut failures = 0usize;
    for path in &paths {
        match check_one(path) {
            Ok(manifest) => eprintln!(
                "  ok   {} — {:?} lane, {} @ {}",
                path.display(),
                manifest.lang,
                manifest.source.repo,
                manifest.source.git_ref
            ),
            Err(e) => {
                eprintln!("  FAIL {} — {e:#}", path.display());
                failures += 1;
            }
        }
    }
    if failures > 0 {
        bail!("{failures} of {} manifest(s) invalid", paths.len());
    }
    eprintln!("All {} manifest(s) valid.", paths.len());
    Ok(())
}

/// Parse one manifest and validate cheap, toolchain-free invariants.
fn check_one(path: &std::path::Path) -> Result<Manifest> {
    let manifest = Manifest::load(path)?;
    // The vendor patch is tracked in-repo, so its existence is checkable in CI.
    // (source.dir lives in gitignored scratch/, so it's intentionally not checked.)
    if let Some(patch) = &manifest.build.vendor_patch
        && !patch.exists()
    {
        bail!("vendor_patch {} does not exist", patch.display());
    }
    Ok(manifest)
}

fn print_usage() {
    eprintln!(
        "conch-build — manifest-driven CLI component builder\n\n\
         Usage:\n  \
         conch-build <name>        Build clis/<name>.toml\n  \
         conch-build <path.toml>   Build a manifest at an explicit path\n  \
         conch-build --list        List available manifests\n  \
         conch-build --check       Validate all manifests (no build)"
    );
}
