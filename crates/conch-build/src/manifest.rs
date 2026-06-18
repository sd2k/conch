//! Per-CLI build manifest schema and loading.
//!
//! A manifest (`clis/<name>.toml`) declares everything the driver needs to turn
//! an upstream CLI's source into a WASI component: which language lane to use,
//! where the source lives, how to patch and build it, and where artifacts land.
//! See ADR #26 and issue #51.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

/// A fully-parsed CLI build manifest.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Command name (e.g. `gh`). Also the registered command name in conch.
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Build lane selecting the toolchain.
    pub lang: Lang,
    /// Upstream source location.
    pub source: Source,
    /// Build configuration.
    pub build: Build,
    /// Component-model settings.
    pub component: Component,
    /// Optional cwasm pre-compilation.
    #[serde(default)]
    pub cwasm: Cwasm,
    /// Where built artifacts are written.
    pub output: Output,
}

/// The language lane used to build a CLI, per ADR #26.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    /// Rust → wasip2 via cargo.
    Rust,
    /// C/C++ → wasip2 via wasi-sdk.
    C,
    /// Go → wasip3 via the experimental Go fork.
    Go,
}

/// Where a CLI's source comes from.
#[derive(Debug, Deserialize)]
pub struct Source {
    /// Upstream repository URL (for provenance/reproducibility).
    pub repo: String,
    /// Pinned git ref (tag or commit) the build targets.
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// Local working copy the driver builds from, relative to the repo root.
    pub dir: PathBuf,
}

/// How to build the CLI within its source tree.
#[derive(Debug, Deserialize)]
pub struct Build {
    /// Package/path to build, relative to the source dir (lane-specific meaning).
    #[serde(default = "default_package")]
    pub package: String,
    /// Optional script (repo-root-relative) applied after dependency vendoring
    /// to stub out code that doesn't compile for the wasm target.
    #[serde(default)]
    pub vendor_patch: Option<PathBuf>,
}

fn default_package() -> String {
    ".".to_string()
}

/// Component-model embedding settings.
#[derive(Debug, Deserialize)]
pub struct Component {
    /// `wasm-tools component embed` world (e.g. `command`).
    pub world: String,
}

/// Optional cwasm (pre-compiled) output settings.
#[derive(Debug, Deserialize, Default)]
pub struct Cwasm {
    /// Whether to also pre-compile the component to `.cwasm`.
    #[serde(default)]
    pub enabled: bool,
    /// Extra `wasmtime compile` flags (e.g. the async/stackful knobs gcx needs).
    #[serde(default)]
    pub flags: Vec<String>,
}

/// Where the driver writes build artifacts.
#[derive(Debug, Deserialize)]
pub struct Output {
    /// Output directory, relative to the repo root.
    pub dir: PathBuf,
}

impl Manifest {
    /// Load and parse a manifest from a TOML file.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading manifest {}", path.display()))?;
        let manifest: Manifest = toml::from_str(&text)
            .with_context(|| format!("parsing manifest {}", path.display()))?;
        Ok(manifest)
    }
}
