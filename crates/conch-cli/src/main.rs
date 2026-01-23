//! Conch CLI - Test harness for the Conch shell
//!
//! Usage:
//!   conch -c "command"     Execute a command string
//!   conch script.sh        Execute a script file
//!   conch                   Read script from stdin

use std::io::{self, Read, Write};

use conch::{ComponentShellExecutor, ResourceLimits};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let script = if args.len() >= 3 && args[1] == "-c" {
        // Inline script: conch -c "echo hello"
        args[2].clone()
    } else if args.len() >= 2 && args[1] != "-c" {
        // Script file: conch script.sh
        match std::fs::read_to_string(&args[1]) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("conch: {}: {}", args[1], e);
                std::process::exit(1);
            }
        }
    } else {
        // Read from stdin
        let mut script = String::new();
        if let Err(e) = io::stdin().read_to_string(&mut script) {
            eprintln!("conch: failed to read stdin: {}", e);
            std::process::exit(1);
        }
        script
    };

    // Create the shell executor
    let executor = match ComponentShellExecutor::embedded() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("conch: failed to initialize shell: {}", e);
            std::process::exit(1);
        }
    };

    // Execute the script
    let limits = ResourceLimits::default();
    let result = match executor.execute(&script, &limits).await {
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
