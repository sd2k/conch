//! wc builtin - word, line, character, and byte count

use std::io::{Read, Write};

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};

pub struct WcCommand;

impl builtins::SimpleCommand for WcCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => {
                Ok("Print newline, word, and byte counts for each FILE.".into())
            }
            builtins::ContentType::ShortUsage => Ok("wc [OPTION]... [FILE]...".into()),
            builtins::ContentType::ShortDescription => {
                Ok("wc - print newline, word, and byte counts".into())
            }
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let mut show_lines = false;
        let mut show_words = false;
        let mut show_chars = false;
        let mut show_bytes = false;
        let mut files = Vec::new();

        // Parse options
        for arg in args.skip(1) {
            let arg = arg.as_ref();
            if arg.starts_with('-') && arg.len() > 1 {
                for c in arg[1..].chars() {
                    match c {
                        'l' => show_lines = true,
                        'w' => show_words = true,
                        'm' => show_chars = true,
                        'c' => show_bytes = true,
                        _ => {
                            writeln!(context.stderr(), "wc: unknown option: -{}", c)?;
                            return Ok(ExecutionResult::new(1));
                        }
                    }
                }
            } else {
                files.push(arg.to_string());
            }
        }

        // Default: show all three
        if !show_lines && !show_words && !show_chars && !show_bytes {
            show_lines = true;
            show_words = true;
            show_bytes = true;
        }

        // If no files, read from stdin
        if files.is_empty() {
            files.push("-".to_string());
        }

        let mut total_lines = 0usize;
        let mut total_words = 0usize;
        let mut total_bytes = 0usize;
        let mut total_chars = 0usize;
        let mut exit_code = 0;

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
                        writeln!(context.stderr(), "wc: {}: {}", file, e)?;
                        exit_code = 1;
                        continue;
                    }
                }
            };

            let counts = count(&contents);

            // Print counts
            let mut parts = Vec::new();
            if show_lines {
                parts.push(format!("{:8}", counts.lines));
                total_lines += counts.lines;
            }
            if show_words {
                parts.push(format!("{:8}", counts.words));
                total_words += counts.words;
            }
            if show_chars {
                parts.push(format!("{:8}", counts.chars));
                total_chars += counts.chars;
            }
            if show_bytes {
                parts.push(format!("{:8}", counts.bytes));
                total_bytes += counts.bytes;
            }

            let display_name = if file == "-" { "" } else { file.as_str() };
            writeln!(context.stdout(), "{} {}", parts.join(""), display_name)?;
        }

        // Print total if multiple files
        if files.len() > 1 {
            let mut parts = Vec::new();
            if show_lines {
                parts.push(format!("{:8}", total_lines));
            }
            if show_words {
                parts.push(format!("{:8}", total_words));
            }
            if show_chars {
                parts.push(format!("{:8}", total_chars));
            }
            if show_bytes {
                parts.push(format!("{:8}", total_bytes));
            }
            writeln!(context.stdout(), "{} total", parts.join(""))?;
        }

        Ok(ExecutionResult::new(exit_code))
    }
}

struct Counts {
    lines: usize,
    words: usize,
    bytes: usize,
    chars: usize,
}

fn count(data: &[u8]) -> Counts {
    let bytes = data.len();
    let lines = data.iter().filter(|&&b| b == b'\n').count();

    // Count words (sequences of non-whitespace)
    let mut words = 0;
    let mut in_word = false;

    for &b in data {
        let is_ws = b.is_ascii_whitespace();
        if in_word && is_ws {
            in_word = false;
        } else if !in_word && !is_ws {
            in_word = true;
            words += 1;
        }
    }

    // Count chars (UTF-8 codepoints)
    let chars = String::from_utf8_lossy(data).chars().count();

    Counts {
        lines,
        words,
        bytes,
        chars,
    }
}
