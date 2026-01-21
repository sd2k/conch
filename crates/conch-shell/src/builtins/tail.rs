//! tail builtin - output the last part of files

use std::io::{Read, Write};

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};

pub struct TailCommand;

impl builtins::SimpleCommand for TailCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => {
                Ok("Print the last 10 lines of each FILE to standard output.".into())
            }
            builtins::ContentType::ShortUsage => Ok("tail [OPTION]... [FILE]...".into()),
            builtins::ContentType::ShortDescription => {
                Ok("tail - output the last part of files".into())
            }
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let mut num_lines: usize = 10;
        let mut num_bytes: Option<usize> = None;
        let mut from_start = false;
        let mut files = Vec::new();

        let args: Vec<String> = args.skip(1).map(|s| s.as_ref().to_string()).collect();
        let mut args_iter = args.iter().peekable();

        while let Some(arg) = args_iter.next() {
            if arg == "-n" || arg == "--lines" {
                if let Some(n) = args_iter.next() {
                    if let Some(rest) = n.strip_prefix('+') {
                        from_start = true;
                        num_lines = rest.parse().unwrap_or(1);
                    } else {
                        num_lines = n.parse().unwrap_or(10);
                    }
                }
            } else if let Some(rest) = arg.strip_prefix("-n") {
                if let Some(rest) = rest.strip_prefix('+') {
                    from_start = true;
                    num_lines = rest.parse().unwrap_or(1);
                } else {
                    num_lines = rest.parse().unwrap_or(10);
                }
            } else if arg == "-c" || arg == "--bytes" {
                if let Some(n) = args_iter.next() {
                    num_bytes = n.parse().ok();
                }
            } else if let Some(rest) = arg.strip_prefix("-c") {
                num_bytes = rest.parse().ok();
            } else if let Some(rest) = arg.strip_prefix('-') {
                // -N shorthand
                if let Ok(n) = rest.parse::<usize>() {
                    num_lines = n;
                }
            } else if let Some(rest) = arg.strip_prefix('+') {
                // +N means from line N
                from_start = true;
                num_lines = rest.parse().unwrap_or(1);
            } else {
                files.push(arg.clone());
            }
        }

        // If no files, read from stdin
        if files.is_empty() {
            files.push("-".to_string());
        }

        let mut exit_code = 0;
        let show_headers = files.len() > 1;

        for (i, file) in files.iter().enumerate() {
            if show_headers {
                if i > 0 {
                    writeln!(context.stdout())?;
                }
                writeln!(context.stdout(), "==> {} <==", file)?;
            }

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
                        writeln!(context.stderr(), "tail: {}: {}", file, e)?;
                        exit_code = 1;
                        continue;
                    }
                }
            };

            if let Some(bytes) = num_bytes {
                // Byte mode
                let mut stdout = context.stdout();
                if from_start {
                    let start = (bytes.saturating_sub(1)).min(contents.len());
                    stdout.write_all(&contents[start..])?;
                } else {
                    let start = contents.len().saturating_sub(bytes);
                    stdout.write_all(&contents[start..])?;
                }
                stdout.flush()?;
            } else {
                // Line mode
                let lines: Vec<&[u8]> = contents.split(|&b| b == b'\n').collect();

                if from_start {
                    // Skip first N-1 lines
                    let start = num_lines.saturating_sub(1);
                    for (idx, line) in lines.iter().enumerate() {
                        if idx >= start {
                            context.stdout().write_all(line)?;
                            if idx < lines.len() - 1 {
                                context.stdout().write_all(b"\n")?;
                            }
                        }
                    }
                } else {
                    // Take last N lines
                    let total_lines = lines.len();
                    let start = if total_lines > num_lines {
                        total_lines - num_lines - 1 // -1 because split adds empty at end
                    } else {
                        0
                    };

                    for (idx, line) in lines.iter().enumerate() {
                        if idx >= start {
                            context.stdout().write_all(line)?;
                            if idx < lines.len() - 1 {
                                context.stdout().write_all(b"\n")?;
                            }
                        }
                    }
                }
            }
        }

        Ok(ExecutionResult::new(exit_code))
    }
}
