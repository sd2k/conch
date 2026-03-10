//! ls builtin - list directory contents

use std::io::Write;

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};

pub struct LsCommand;

impl builtins::SimpleCommand for LsCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => Ok("List directory contents.".into()),
            builtins::ContentType::ShortUsage => Ok("ls [-la] [file...]".into()),
            builtins::ContentType::ShortDescription => Ok("ls - list directory contents".into()),
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let mut show_all = false;
        let mut long_format = false;
        let mut one_per_line = false;
        let mut paths = Vec::new();
        let mut parsing_options = true;

        for arg in args.skip(1) {
            let arg = arg.as_ref();
            if parsing_options && arg == "--" {
                parsing_options = false;
            } else if parsing_options && arg.starts_with('-') && arg.len() > 1 {
                for c in arg[1..].chars() {
                    match c {
                        'a' | 'A' => show_all = true,
                        'l' => long_format = true,
                        '1' => one_per_line = true,
                        'h' | 'F' | 'r' | 't' | 'S' | 'R' | 'd' => {
                            // Common options we ignore for simplicity
                        }
                        _ => {
                            writeln!(context.stderr(), "ls: unknown option: -{}", c)?;
                            return Ok(ExecutionResult::new(1));
                        }
                    }
                }
            } else {
                paths.push(arg.to_string());
            }
        }

        if paths.is_empty() {
            paths.push(".".to_string());
        }

        let mut exit_code = 0;
        let multiple_paths = paths.len() > 1;

        for (i, path_str) in paths.iter().enumerate() {
            let path = std::path::Path::new(path_str);

            if !path.exists() {
                writeln!(
                    context.stderr(),
                    "ls: cannot access '{}': No such file or directory",
                    path_str
                )?;
                exit_code = 1;
                continue;
            }

            if path.is_file() {
                // Just print the file name
                if long_format {
                    print_long_entry(&context, path)?;
                } else {
                    writeln!(context.stdout(), "{}", path_str)?;
                }
                continue;
            }

            // It's a directory
            if multiple_paths {
                if i > 0 {
                    writeln!(context.stdout())?;
                }
                writeln!(context.stdout(), "{}:", path_str)?;
            }

            let entries = match std::fs::read_dir(path) {
                Ok(entries) => entries,
                Err(e) => {
                    writeln!(
                        context.stderr(),
                        "ls: cannot open directory '{}': {}",
                        path_str,
                        e
                    )?;
                    exit_code = 1;
                    continue;
                }
            };

            let mut names: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name();
                    let name_str = name.to_string_lossy();
                    show_all || !name_str.starts_with('.')
                })
                .collect();

            names.sort_by_key(|a| a.file_name());

            if long_format {
                for entry in &names {
                    print_long_entry(&context, &entry.path())?;
                }
            } else if one_per_line {
                for entry in &names {
                    writeln!(context.stdout(), "{}", entry.file_name().to_string_lossy())?;
                }
            } else {
                // Simple format - space separated
                let name_strs: Vec<_> = names
                    .iter()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                writeln!(context.stdout(), "{}", name_strs.join("  "))?;
            }
        }

        context.stdout().flush()?;
        Ok(ExecutionResult::new(exit_code))
    }
}

fn print_long_entry<SE: ShellExtensions>(
    context: &ExecutionContext<'_, SE>,
    path: &std::path::Path,
) -> Result<(), brush_core::Error> {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => {
            writeln!(context.stdout(), "?????????? ? ? ? ? ? {}", path.display())?;
            return Ok(());
        }
    };

    let file_type = if metadata.is_dir() {
        'd'
    } else if metadata.is_symlink() {
        'l'
    } else {
        '-'
    };

    // Simplified permissions (WASM VFS doesn't have real permissions)
    let perms = if metadata.is_dir() {
        "rwxr-xr-x"
    } else {
        "rw-r--r--"
    };

    let size = metadata.len();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();

    writeln!(
        context.stdout(),
        "{}{} 1 user user {:>8} Jan  1 00:00 {}",
        file_type,
        perms,
        size,
        name
    )?;

    Ok(())
}
