//! Build lanes: language-keyed strategies for producing a WASI component.
//!
//! Each lane consumes a [`Manifest`] and emits a component into the manifest's
//! output dir. The Rust and C lanes are stubs pending their compile spikes
//! (#52, #79); the Go lane is implemented (it backs the `gh` and `gcx` builds).

pub mod c;
pub mod go;
pub mod rust;

use anyhow::Result;

use crate::manifest::{Lang, Manifest};

/// Dispatch a build to the lane selected by the manifest's `lang`.
pub fn build(manifest: &Manifest) -> Result<()> {
    match manifest.lang {
        Lang::Go => go::build(manifest),
        Lang::Rust => rust::build(manifest),
        Lang::C => c::build(manifest),
    }
}

/// Run a subprocess, returning an error if it exits non-zero.
///
/// Shared by the lanes; logs the command before running it.
pub(crate) fn run(
    program: &str,
    args: &[&str],
    cwd: &std::path::Path,
    envs: &[(&str, &str)],
) -> Result<()> {
    use anyhow::{Context, bail};

    let pretty_env: String = envs
        .iter()
        .map(|(k, v)| format!("{k}={v} "))
        .collect::<String>();
    eprintln!(
        "  $ (cd {}) {pretty_env}{program} {}",
        cwd.display(),
        args.join(" ")
    );

    let mut cmd = std::process::Command::new(program);
    cmd.args(args).current_dir(cwd);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .with_context(|| format!("failed to spawn `{program}` (is it on PATH?)"))?;
    if !status.success() {
        bail!("`{program}` exited with {status}");
    }
    Ok(())
}
