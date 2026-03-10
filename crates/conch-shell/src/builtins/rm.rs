//! rm builtin - remove files and directories

use std::io::Write;
use std::path::Path;

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};

/// Recursively remove a directory and all its contents.
/// This is implemented manually because std::fs::remove_dir_all doesn't work
/// properly with the WASI preview2-shim VFS (directory iteration issues).
fn remove_dir_recursive(path: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            remove_dir_recursive(&entry_path)?;
        } else {
            std::fs::remove_file(&entry_path)?;
        }
    }
    std::fs::remove_dir(path)
}

pub struct RmCommand;

impl builtins::SimpleCommand for RmCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => Ok("Remove files or directories.".into()),
            builtins::ContentType::ShortUsage => Ok("rm [-rf] file...".into()),
            builtins::ContentType::ShortDescription => {
                Ok("rm - remove files or directories".into())
            }
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let mut recursive = false;
        let mut force = false;
        let mut files = Vec::new();
        let mut parsing_options = true;

        for arg in args.skip(1) {
            let arg = arg.as_ref();
            if parsing_options && arg == "--" {
                parsing_options = false;
            } else if parsing_options && arg == "-r" || arg == "-R" || arg == "--recursive" {
                recursive = true;
            } else if parsing_options && arg == "-f" || arg == "--force" {
                force = true;
            } else if parsing_options && arg == "-rf" || arg == "-fr" {
                recursive = true;
                force = true;
            } else if parsing_options && arg.starts_with('-') && arg.len() > 1 {
                for c in arg[1..].chars() {
                    match c {
                        'r' | 'R' => recursive = true,
                        'f' => force = true,
                        'i' | 'I' | 'v' | 'd' => {
                            // Interactive, verbose, directory - ignore for simplicity
                        }
                        _ => {
                            writeln!(context.stderr(), "rm: unknown option: -{}", c)?;
                            return Ok(ExecutionResult::new(1));
                        }
                    }
                }
            } else {
                files.push(arg.to_string());
            }
        }

        if files.is_empty() {
            if !force {
                writeln!(context.stderr(), "rm: missing operand")?;
                return Ok(ExecutionResult::new(1));
            }
            return Ok(ExecutionResult::new(0));
        }

        let mut exit_code = 0;

        for file in &files {
            let path = std::path::Path::new(file);

            if !path.exists() {
                if !force {
                    writeln!(
                        context.stderr(),
                        "rm: cannot remove '{}': No such file or directory",
                        file
                    )?;
                    exit_code = 1;
                }
                continue;
            }

            let result = if path.is_dir() {
                if recursive {
                    // Implement recursive deletion manually because remove_dir_all
                    // doesn't work properly with WASI preview2-shim VFS
                    remove_dir_recursive(path)
                } else {
                    writeln!(
                        context.stderr(),
                        "rm: cannot remove '{}': Is a directory",
                        file
                    )?;
                    exit_code = 1;
                    continue;
                }
            } else {
                std::fs::remove_file(path)
            };

            if let Err(e) = result
                && !force
            {
                writeln!(context.stderr(), "rm: cannot remove '{}': {}", file, e)?;
                exit_code = 1;
            }
        }

        Ok(ExecutionResult::new(exit_code))
    }
}
