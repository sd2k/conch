//! grep builtin - search for patterns in files

use std::io::{BufRead, Read, Write};

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};

pub struct GrepCommand;

impl builtins::SimpleCommand for GrepCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => Ok("Search for PATTERN in each FILE.".into()),
            builtins::ContentType::ShortUsage => Ok("grep [OPTION]... PATTERN [FILE]...".into()),
            builtins::ContentType::ShortDescription => {
                Ok("grep - print lines matching a pattern".into())
            }
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        mut context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let args: Vec<String> = args.skip(1).map(|s| s.as_ref().to_string()).collect();

        let opts = match GrepOpts::parse(&args) {
            Ok(o) => o,
            Err(e) => {
                writeln!(context.stderr(), "grep: {}", e)?;
                return Ok(ExecutionResult::new(2));
            }
        };

        let regex = match regex_lite::Regex::new(&opts.pattern) {
            Ok(r) => r,
            Err(e) => {
                writeln!(context.stderr(), "grep: invalid regex: {}", e)?;
                return Ok(ExecutionResult::new(2));
            }
        };

        let mut matched = false;
        let mut match_count = 0;

        if opts.files.is_empty() {
            // Read from stdin
            let mut stdin = context.stdin();
            let mut buf = Vec::new();
            stdin.read_to_end(&mut buf)?;
            let result = grep_reader(&buf, &regex, None, &opts, &mut context, &mut match_count)?;
            matched |= result;
        } else {
            let show_filename = opts.files.len() > 1 || opts.with_filename;

            for file_pattern in &opts.files {
                match std::fs::read(file_pattern) {
                    Ok(contents) => {
                        let filename = if show_filename {
                            Some(file_pattern.as_str())
                        } else {
                            None
                        };
                        let result = grep_reader(
                            &contents,
                            &regex,
                            filename,
                            &opts,
                            &mut context,
                            &mut match_count,
                        )?;
                        matched |= result;

                        if opts.files_only && result {
                            writeln!(context.stdout(), "{}", file_pattern)?;
                        }
                    }
                    Err(e) => {
                        if !opts.silent {
                            writeln!(context.stderr(), "grep: {}: {}", file_pattern, e)?;
                        }
                    }
                }
            }
        }

        if opts.count_only {
            writeln!(context.stdout(), "{}", match_count)?;
        }

        // Ensure output is flushed
        context.stdout().flush()?;

        let exit_code = if matched { 0 } else { 1 };
        Ok(ExecutionResult::new(exit_code))
    }
}

fn grep_reader<SE: ShellExtensions>(
    input: &[u8],
    regex: &regex_lite::Regex,
    filename: Option<&str>,
    opts: &GrepOpts,
    context: &mut ExecutionContext<'_, SE>,
    match_count: &mut usize,
) -> Result<bool, brush_core::Error> {
    let mut matched = false;
    let mut line_number = 0;

    for line in input.lines() {
        line_number += 1;

        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        let is_match = regex.is_match(&line);
        let is_match = if opts.invert { !is_match } else { is_match };

        if is_match {
            matched = true;
            *match_count += 1;

            if opts.files_only || opts.count_only || opts.silent {
                continue;
            }

            // Build output line
            if let Some(f) = filename {
                write!(context.stdout(), "{}:", f)?;
            }

            if opts.line_number {
                write!(context.stdout(), "{}:", line_number)?;
            }

            writeln!(context.stdout(), "{}", line)?;

            // Handle max count
            if let Some(max) = opts.max_count {
                if *match_count >= max {
                    break;
                }
            }
        }
    }

    Ok(matched)
}

struct GrepOpts {
    pattern: String,
    files: Vec<String>,
    invert: bool,
    ignore_case: bool,
    line_number: bool,
    count_only: bool,
    files_only: bool,
    with_filename: bool,
    silent: bool,
    max_count: Option<usize>,
}

impl GrepOpts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut opts = GrepOpts {
            pattern: String::new(),
            files: Vec::new(),
            invert: false,
            ignore_case: false,
            line_number: false,
            count_only: false,
            files_only: false,
            with_filename: false,
            silent: false,
            max_count: None,
        };

        let mut positional = Vec::new();
        let mut args_iter = args.iter().peekable();

        while let Some(arg) = args_iter.next() {
            if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut i = 0;

                while i < chars.len() {
                    match chars[i] {
                        'v' => opts.invert = true,
                        'i' => opts.ignore_case = true,
                        'n' => opts.line_number = true,
                        'c' => opts.count_only = true,
                        'l' => opts.files_only = true,
                        'H' => opts.with_filename = true,
                        'q' => opts.silent = true,
                        'e' => {
                            // -e PATTERN (pattern follows)
                            if i + 1 < chars.len() {
                                // Pattern is rest of this arg
                                opts.pattern = chars[i + 1..].iter().collect();
                                i = chars.len();
                            } else if let Some(p) = args_iter.next() {
                                opts.pattern = p.clone();
                            } else {
                                return Err("option requires an argument -- 'e'".to_string());
                            }
                        }
                        'm' => {
                            // -m NUM
                            if let Some(n) = args_iter.next() {
                                opts.max_count = n.parse().ok();
                            }
                        }
                        _ => return Err(format!("unknown option: -{}", chars[i])),
                    }
                    i += 1;
                }
            } else if arg.starts_with("--") {
                match arg.as_str() {
                    "--invert-match" => opts.invert = true,
                    "--ignore-case" => opts.ignore_case = true,
                    "--line-number" => opts.line_number = true,
                    "--count" => opts.count_only = true,
                    "--files-with-matches" => opts.files_only = true,
                    "--with-filename" => opts.with_filename = true,
                    "--quiet" | "--silent" => opts.silent = true,
                    _ => return Err(format!("unknown option: {}", arg)),
                }
            } else {
                positional.push(arg.clone());
            }
        }

        // First positional is pattern (if not set via -e)
        if opts.pattern.is_empty() {
            if positional.is_empty() {
                return Err("missing pattern".to_string());
            }
            opts.pattern = positional.remove(0);
        }

        // Case insensitive
        if opts.ignore_case {
            opts.pattern = format!("(?i){}", opts.pattern);
        }

        opts.files = positional;
        Ok(opts)
    }
}
