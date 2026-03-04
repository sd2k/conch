//! touch builtin - create files or update timestamps

use std::io::Write;

use brush_core::{builtins, error, ExecutionContext, ExecutionResult, ShellExtensions};

pub struct TouchCommand;

impl builtins::SimpleCommand for TouchCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => {
                Ok("Update file access and modification times, creating files if they don't exist."
                    .into())
            }
            builtins::ContentType::ShortUsage => Ok("touch [-c] file...".into()),
            builtins::ContentType::ShortDescription => {
                Ok("touch - change file timestamps".into())
            }
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let mut no_create = false;
        let mut files = Vec::new();
        let mut parsing_options = true;

        for arg in args.skip(1) {
            let arg = arg.as_ref();
            if parsing_options && arg == "--" {
                parsing_options = false;
            } else if parsing_options && arg == "-c" {
                no_create = true;
            } else if parsing_options && arg.starts_with('-') && arg.len() > 1 {
                for c in arg[1..].chars() {
                    match c {
                        'c' => no_create = true,
                        'a' | 'm' | 'r' | 't' | 'd' => {
                            // These options require arguments or have complex behavior
                            // For simplicity, we ignore them
                        }
                        _ => {
                            writeln!(context.stderr(), "touch: unknown option: -{}", c)?;
                            return Ok(ExecutionResult::new(1));
                        }
                    }
                }
            } else {
                files.push(arg.to_string());
            }
        }

        if files.is_empty() {
            writeln!(context.stderr(), "touch: missing file operand")?;
            return Ok(ExecutionResult::new(1));
        }

        let mut exit_code = 0;

        for file in &files {
            let path = std::path::Path::new(file);

            if path.exists() {
                // File exists - update timestamps
                // In WASM VFS, this is a no-op since we don't track real timestamps
                // But we try to "touch" it by opening and closing
                if let Err(e) = std::fs::OpenOptions::new().write(true).open(path) {
                    writeln!(context.stderr(), "touch: cannot touch '{}': {}", file, e)?;
                    exit_code = 1;
                }
            } else if !no_create {
                // Create the file
                if let Err(e) = std::fs::File::create(path) {
                    writeln!(context.stderr(), "touch: cannot touch '{}': {}", file, e)?;
                    exit_code = 1;
                }
            }
            // If -c and file doesn't exist, do nothing (success)
        }

        Ok(ExecutionResult::new(exit_code))
    }
}
