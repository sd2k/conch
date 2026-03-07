//! Conch CLI - Test harness for the Conch shell
//!
//! Usage:
//!   conch -c "command"                    Execute a command string
//!   conch script.sh                       Execute a script file
//!   conch                                 Read script from stdin
//!   conch --commands-dir ./cmds -c "..."  Register WASM components as commands

use std::io::{self, Read, Write};
use std::path::PathBuf;

use conch::{ComponentRegistry, ResourceLimits, Shell};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut commands_dir: Option<PathBuf> = None;
    let mut script_source: Option<ScriptSource> = None;
    let mut i = 1;

    // Parse arguments
    while i < args.len() {
        match args[i].as_str() {
            "--commands-dir" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("conch: --commands-dir requires an argument");
                    std::process::exit(1);
                }
                commands_dir = Some(PathBuf::from(&args[i]));
            }
            "-c" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("conch: -c requires an argument");
                    std::process::exit(1);
                }
                script_source = Some(ScriptSource::Inline(args[i].clone()));
            }
            arg if script_source.is_none() => {
                script_source = Some(ScriptSource::File(PathBuf::from(arg)));
            }
            _ => {
                eprintln!("conch: unexpected argument: {}", args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let script = match script_source {
        Some(ScriptSource::Inline(s)) => s,
        Some(ScriptSource::File(path)) => match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("conch: {}: {}", path.display(), e);
                std::process::exit(1);
            }
        },
        None => {
            let mut script = String::new();
            if let Err(e) = io::stdin().read_to_string(&mut script) {
                eprintln!("conch: failed to read stdin: {}", e);
                std::process::exit(1);
            }
            script
        }
    };

    // Build the shell with optional component registry
    let mut builder = Shell::builder();

    if let Some(dir) = commands_dir {
        let registry = match load_commands_from_dir(&dir) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "conch: failed to load commands from {}: {}",
                    dir.display(),
                    e
                );
                std::process::exit(1);
            }
        };
        if !registry.is_empty() {
            eprintln!(
                "conch: registered {} command(s) from {}",
                registry.len(),
                dir.display()
            );
        }
        builder = builder.component_registry(registry);
    }

    let mut shell = match builder.build().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("conch: failed to initialize shell: {}", e);
            std::process::exit(1);
        }
    };

    // Execute the script
    let limits = ResourceLimits::default();
    let result = match shell.execute(&script, &limits).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("conch: execution error: {}", e);
            std::process::exit(1);
        }
    };

    // Write output
    io::stdout().write_all(&result.stdout).ok();
    io::stderr().write_all(&result.stderr).ok();

    std::process::exit(result.exit_code);
}

enum ScriptSource {
    Inline(String),
    File(PathBuf),
}

/// Load all `.wasm` files from a directory as commands.
///
/// Each file `foo.wasm` is registered as command `foo`.
fn load_commands_from_dir(dir: &std::path::Path) -> Result<ComponentRegistry, String> {
    let mut registry = ComponentRegistry::new();

    let entries = std::fs::read_dir(dir).map_err(|e| format!("failed to read directory: {e}"))?;

    // Collect paths, sorted so .cwasm comes after .wasm and overwrites it
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("wasm" | "cwasm")))
        .collect();
    paths.sort();

    for path in paths {
        let cmd_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("invalid filename: {}", path.display()))?
            .to_string();

        let is_cwasm = path.extension().and_then(|e| e.to_str()) == Some("cwasm");

        // Skip .wasm if we already have a .cwasm for this command
        if !is_cwasm && registry.contains(&cmd_name) {
            continue;
        }

        let bytes =
            std::fs::read(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;

        eprintln!("conch: loaded command '{cmd_name}' from {}", path.display());
        if is_cwasm {
            registry.register_cwasm(cmd_name, bytes);
        } else {
            registry.register_wasm(cmd_name, bytes);
        }
    }

    Ok(registry)
}
