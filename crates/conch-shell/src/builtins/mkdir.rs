//! mkdir builtin - create directories

use std::io::Write;

use brush_core::{builtins, error, ExecutionContext, ExecutionResult, ShellExtensions};

pub struct MkdirCommand;

impl builtins::SimpleCommand for MkdirCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => {
                Ok("Create directories if they do not already exist.".into())
            }
            builtins::ContentType::ShortUsage => Ok("mkdir [-p] directory...".into()),
            builtins::ContentType::ShortDescription => {
                Ok("mkdir - create directories".into())
            }
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let mut parents = false;
        let mut dirs = Vec::new();
        let mut parsing_options = true;

        // Parse arguments
        for arg in args.skip(1) {
            let arg = arg.as_ref();
            if parsing_options && arg == "--" {
                parsing_options = false;
            } else if parsing_options && arg == "-p" {
                parents = true;
            } else if parsing_options && arg.starts_with('-') && arg.len() > 1 {
                // Handle combined options like -pv
                for c in arg[1..].chars() {
                    match c {
                        'p' => parents = true,
                        'v' => {} // Ignore verbose flag
                        'm' => {
                            // Mode - skip for now, would need next arg
                            writeln!(context.stderr(), "mkdir: -m option not supported")?;
                            return Ok(ExecutionResult::new(1));
                        }
                        _ => {
                            writeln!(context.stderr(), "mkdir: unknown option: -{}", c)?;
                            return Ok(ExecutionResult::new(1));
                        }
                    }
                }
            } else {
                dirs.push(arg.to_string());
            }
        }

        if dirs.is_empty() {
            writeln!(context.stderr(), "mkdir: missing operand")?;
            return Ok(ExecutionResult::new(1));
        }

        let mut exit_code = 0;

        for dir in &dirs {
            let path = std::path::Path::new(dir);

            let result = if parents {
                std::fs::create_dir_all(path)
            } else {
                std::fs::create_dir(path)
            };

            if let Err(e) = result {
                // With -p, ignore "already exists" errors for directories
                if parents && e.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() {
                    continue;
                }
                writeln!(
                    context.stderr(),
                    "mkdir: cannot create directory '{}': {}",
                    dir, e
                )?;
                exit_code = 1;
            }
        }

        Ok(ExecutionResult::new(exit_code))
    }
}
