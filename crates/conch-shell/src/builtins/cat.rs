//! cat builtin - concatenate and display files

use std::io::{Read, Write};

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};

pub struct CatCommand;

impl builtins::SimpleCommand for CatCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => {
                Ok("Concatenate files and print to standard output.".into())
            }
            builtins::ContentType::ShortUsage => Ok("cat [OPTION]... [FILE]...".into()),
            builtins::ContentType::ShortDescription => {
                Ok("cat - concatenate files and print on stdout".into())
            }
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let mut show_line_numbers = false;
        let mut show_ends = false;
        let mut squeeze_blank = false;
        let mut files = Vec::new();

        // Parse arguments
        for arg in args.skip(1) {
            let arg = arg.as_ref();
            if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                for c in arg[1..].chars() {
                    match c {
                        'n' => show_line_numbers = true,
                        'E' => show_ends = true,
                        's' => squeeze_blank = true,
                        _ => {
                            writeln!(context.stderr(), "cat: unknown option: -{}", c)?;
                            return Ok(ExecutionResult::new(1));
                        }
                    }
                }
            } else if arg == "-" {
                files.push("-".to_string());
            } else {
                files.push(arg.to_string());
            }
        }

        // If no files specified, read from stdin
        if files.is_empty() {
            files.push("-".to_string());
        }

        let mut exit_code = 0;
        let mut line_number = 1;
        let mut last_was_blank = false;

        for file in &files {
            let contents = if file == "-" {
                // Read from stdin
                let mut stdin = context.stdin();
                let mut buf = Vec::new();
                stdin.read_to_end(&mut buf)?;
                buf
            } else {
                match std::fs::read(file) {
                    Ok(data) => data,
                    Err(e) => {
                        writeln!(context.stderr(), "cat: {}: {}", file, e)?;
                        exit_code = 1;
                        continue;
                    }
                }
            };

            if !show_line_numbers && !show_ends && !squeeze_blank {
                // Fast path - just write the contents
                context.stdout().write_all(&contents)?;
            } else {
                // Line-by-line processing
                for line in contents.split(|&b| b == b'\n') {
                    let is_blank = line.is_empty() || line.iter().all(|&b| b.is_ascii_whitespace());

                    if squeeze_blank && is_blank && last_was_blank {
                        continue;
                    }
                    last_was_blank = is_blank;

                    if show_line_numbers {
                        write!(context.stdout(), "{:6}\t", line_number)?;
                        line_number += 1;
                    }

                    context.stdout().write_all(line)?;

                    if show_ends {
                        context.stdout().write_all(b"$")?;
                    }

                    context.stdout().write_all(b"\n")?;
                }
            }
        }

        // Ensure output is flushed - required for hybrid VFS mode
        context.stdout().flush()?;

        Ok(ExecutionResult::new(exit_code))
    }
}
