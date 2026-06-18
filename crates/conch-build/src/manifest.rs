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
    /// Component-model settings (Go lane only; defaults for other lanes).
    #[serde(default)]
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

/// How the C lane builds a CLI.
#[derive(Debug, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CBuildSystem {
    /// A single `clang` invocation over an explicit source list
    /// (amalgamation-style, e.g. sqlite3). The default.
    #[default]
    Single,
    /// Configure + build a CMake project with the wasi-sdk toolchain file
    /// (e.g. curl). Uses `cmake_flags` and `artifact`.
    Cmake,
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
    /// Used by the Go lane (`go build <package>`).
    #[serde(default = "default_package")]
    pub package: String,
    /// Optional script (repo-root-relative) applied after dependency vendoring
    /// (Go lane) or before compilation (C lane) to stub out code that doesn't
    /// compile for the wasm target.
    #[serde(default)]
    pub vendor_patch: Option<PathBuf>,
    /// C lane: which build system to use (`single` clang call vs `cmake`).
    #[serde(default)]
    pub system: CBuildSystem,
    /// C lane (`single`): source files to compile, relative to the source dir
    /// (e.g. the SQLite amalgamation's `shell.c`, `sqlite3.c`).
    #[serde(default)]
    pub sources: Vec<PathBuf>,
    /// C lane: compiler flags — defines, optimization, feature toggles
    /// (e.g. `-O2`, `-DSQLITE_THREADSAFE=0`). For `cmake` builds these are
    /// passed via `CMAKE_C_FLAGS`.
    #[serde(default)]
    pub cflags: Vec<String>,
    /// C lane: linker flags (e.g. wasi-sdk emulation libs like
    /// `-lwasi-emulated-signal`). For `cmake` builds these go via
    /// `CMAKE_EXE_LINKER_FLAGS`.
    #[serde(default)]
    pub link_flags: Vec<String>,
    /// C lane (`cmake`): extra `-D…` flags for the `cmake` configure step.
    #[serde(default)]
    pub cmake_flags: Vec<String>,
    /// C lane (`cmake`): built artifact path relative to the CMake build dir
    /// (e.g. `src/curl`); copied to `<output>/component.wasm`.
    #[serde(default)]
    pub artifact: Option<PathBuf>,
    /// Rust lane: cargo binary to build (`cargo build --bin <bin>`); the wasm
    /// artifact is `<bin>.wasm`. Required for the Rust lane.
    #[serde(default)]
    pub bin: Option<String>,
    /// Rust lane: extra `cargo build` flags (e.g. `--no-default-features`,
    /// `--features foo`).
    #[serde(default)]
    pub cargo_flags: Vec<String>,
}

fn default_package() -> String {
    ".".to_string()
}

/// Component-model embedding settings.
///
/// Used by the Go lane (its `wasm-tools component embed` step). The C lane emits
/// a component directly from clang, so it ignores this; the table is optional
/// and defaults to `world = "command"` for schema uniformity.
#[derive(Debug, Deserialize)]
pub struct Component {
    /// `wasm-tools component embed` world (e.g. `command`).
    #[serde(default = "default_world")]
    pub world: String,
}

fn default_world() -> String {
    "command".to_string()
}

impl Default for Component {
    fn default() -> Self {
        Self {
            world: default_world(),
        }
    }
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A C-lane manifest parses its `[build]` C fields and may omit
    /// `[component]` entirely (it defaults to `world = "command"`).
    #[test]
    fn parses_c_lane_manifest() {
        let toml = r#"
            name = "sqlite3"
            lang = "c"
            [source]
            repo = "https://example.com/sqlite.zip"
            ref = "3.53.2"
            dir = "scratch/sqlite"
            [build]
            sources = ["shell.c", "sqlite3.c"]
            cflags = ["-O2", "-DSQLITE_THREADSAFE=0"]
            link_flags = ["-lwasi-emulated-signal"]
            [output]
            dir = "scratch/sqlite-component"
        "#;
        let m: Manifest = toml::from_str(toml).expect("C manifest should parse");
        assert_eq!(m.lang, Lang::C);
        assert_eq!(
            m.build.sources,
            vec![PathBuf::from("shell.c"), PathBuf::from("sqlite3.c")]
        );
        assert_eq!(m.build.cflags, ["-O2", "-DSQLITE_THREADSAFE=0"]);
        assert_eq!(m.build.link_flags, ["-lwasi-emulated-signal"]);
        // No [component] table → defaulted.
        assert_eq!(m.component.world, "command");
        // Default build system is `single`.
        assert_eq!(m.build.system, CBuildSystem::Single);
    }

    /// A C/CMake manifest (curl-style) parses `system = "cmake"` plus its
    /// cmake_flags/artifact fields.
    #[test]
    fn parses_c_cmake_manifest() {
        let toml = r#"
            name = "curl"
            lang = "c"
            [source]
            repo = "https://github.com/curl/curl.git"
            ref = "curl-8_20_0"
            dir = "scratch/curl"
            [build]
            system = "cmake"
            artifact = "src/curl"
            cflags = ["-DPOLLPRI=0"]
            cmake_flags = ["-DCURL_ENABLE_SSL=OFF", "-DBUILD_CURL_EXE=ON"]
            [output]
            dir = "scratch/curl-component"
        "#;
        let m: Manifest = toml::from_str(toml).expect("cmake manifest should parse");
        assert_eq!(m.build.system, CBuildSystem::Cmake);
        assert_eq!(m.build.artifact, Some(PathBuf::from("src/curl")));
        assert_eq!(
            m.build.cmake_flags,
            ["-DCURL_ENABLE_SSL=OFF", "-DBUILD_CURL_EXE=ON"]
        );
        // sources is unused in cmake mode and defaults empty.
        assert!(m.build.sources.is_empty());
    }

    /// A Rust-lane manifest (ripgrep-style) parses its `bin`/`cargo_flags`.
    #[test]
    fn parses_rust_lane_manifest() {
        let toml = r#"
            name = "rg"
            lang = "rust"
            [source]
            repo = "https://github.com/BurntSushi/ripgrep.git"
            ref = "15.1.0"
            dir = "scratch/ripgrep"
            [build]
            bin = "rg"
            cargo_flags = ["--no-default-features"]
            [output]
            dir = "scratch/ripgrep-component"
        "#;
        let m: Manifest = toml::from_str(toml).expect("Rust manifest should parse");
        assert_eq!(m.lang, Lang::Rust);
        assert_eq!(m.build.bin.as_deref(), Some("rg"));
        assert_eq!(m.build.cargo_flags, ["--no-default-features"]);
    }

    /// A Go-lane manifest still parses, and its C-only `[build]` fields default
    /// to empty (so the schema change is backward-compatible).
    #[test]
    fn go_lane_manifest_leaves_c_fields_empty() {
        let toml = r#"
            name = "gh"
            lang = "go"
            [source]
            repo = "https://github.com/cli/cli.git"
            ref = "v2.87.3"
            dir = "scratch/gh-cli"
            [build]
            package = "./cmd/gh"
            [component]
            world = "command"
            [output]
            dir = "scratch/gh-component"
        "#;
        let m: Manifest = toml::from_str(toml).expect("Go manifest should parse");
        assert_eq!(m.lang, Lang::Go);
        assert_eq!(m.build.package, "./cmd/gh");
        assert!(m.build.sources.is_empty());
        assert!(m.build.cflags.is_empty());
    }
}
