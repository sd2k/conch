//! head builtin - output the first part of files

use std::io::{Read, Write};

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};

pub struct HeadCommand;

impl builtins::SimpleCommand for HeadCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => {
                Ok("Print the first 10 lines of each FILE to standard output.".into())
            }
            builtins::ContentType::ShortUsage => Ok("head [OPTION]... [FILE]...".into()),
            builtins::ContentType::ShortDescription => {
                Ok("head - output the first part of files".into())
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
        let mut files = Vec::new();

        let args: Vec<String> = args.skip(1).map(|s| s.as_ref().to_string()).collect();
        let mut args_iter = args.iter().peekable();

        while let Some(arg) = args_iter.next() {
            if arg == "-n" || arg == "--lines" {
                if let Some(n) = args_iter.next() {
                    num_lines = n.parse().unwrap_or(10);
                }
            } else if let Some(suffix) = arg.strip_prefix("-n") {
                num_lines = suffix.parse().unwrap_or(10);
            } else if arg == "-c" || arg == "--bytes" {
                if let Some(n) = args_iter.next() {
                    num_bytes = n.parse().ok();
                }
            } else if let Some(suffix) = arg.strip_prefix("-c") {
                num_bytes = suffix.parse().ok();
            } else if let Some(suffix) = arg.strip_prefix('-') {
                // -N shorthand for -n N
                if let Ok(n) = suffix.parse::<usize>() {
                    num_lines = n;
                }
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
                        writeln!(context.stderr(), "head: {}: {}", file, e)?;
                        exit_code = 1;
                        continue;
                    }
                }
            };

            if let Some(bytes) = num_bytes {
                // Byte mode
                let end = bytes.min(contents.len());
                let mut stdout = context.stdout();
                stdout.write_all(&contents[..end])?;
                stdout.flush()?;
            } else {
                // Line mode
                let mut line_count = 0;
                for &byte in contents.iter() {
                    context.stdout().write_all(&[byte])?;
                    if byte == b'\n' {
                        line_count += 1;
                        if line_count >= num_lines {
                            break;
                        }
                    }
                }
            }
        }

        Ok(ExecutionResult::new(exit_code))
    }
}
