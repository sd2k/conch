//! jq builtin - JSON processor using jaq
//!
//! Full jq implementation using the jaq library.

use std::io::{Read, Write};

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Compiler, Ctx, Vars, data, unwrap_valr};
use jaq_json::Val;

pub struct JqCommand;

impl builtins::SimpleCommand for JqCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => Ok("Command-line JSON processor.".into()),
            builtins::ContentType::ShortUsage => Ok("jq [OPTIONS] FILTER [FILE]".into()),
            builtins::ContentType::ShortDescription => {
                Ok("jq - command-line JSON processor".into())
            }
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        mut context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let args: Vec<String> = args.skip(1).map(|s| s.as_ref().to_string()).collect();

        let opts = match JqOpts::parse(&args) {
            Ok(o) => o,
            Err(e) => {
                writeln!(context.stderr(), "jq: {}", e)?;
                return Ok(ExecutionResult::new(2));
            }
        };

        // Read input
        let input_bytes = if opts.files.is_empty() {
            // Read from stdin
            let mut stdin = context.stdin();
            let mut buf = Vec::new();
            stdin.read_to_end(&mut buf)?;
            buf
        } else {
            match std::fs::read(&opts.files[0]) {
                Ok(data) => data,
                Err(e) => {
                    writeln!(context.stderr(), "jq: {}: {}", opts.files[0], e)?;
                    return Ok(ExecutionResult::new(1));
                }
            }
        };

        // Compile the filter
        let program = File {
            code: opts.filter.as_str(),
            path: (),
        };

        let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
        let arena = Arena::default();

        let modules = match loader.load(&arena, program) {
            Ok(m) => m,
            Err(errs) => {
                for err in errs {
                    writeln!(context.stderr(), "jq: parse error: {:?}", err)?;
                }
                return Ok(ExecutionResult::new(3));
            }
        };

        let filter = match Compiler::<_, data::JustLut<Val>>::default()
            .with_funs(jaq_std::funs().chain(jaq_json::funs()))
            .compile(modules)
        {
            Ok(f) => f,
            Err(errs) => {
                for err in errs {
                    writeln!(context.stderr(), "jq: compile error: {:?}", err)?;
                }
                return Ok(ExecutionResult::new(3));
            }
        };

        // Handle null input
        if opts.null_input {
            return run_filter(&filter, Val::Null, &opts, &mut context);
        }

        // Handle raw input mode
        if opts.raw_input {
            let text = String::from_utf8_lossy(&input_bytes);
            for line in text.lines() {
                let val = Val::from(line.to_string());
                let result = run_filter(&filter, val, &opts, &mut context)?;
                if !result.is_success() {
                    return Ok(result);
                }
            }
            return Ok(ExecutionResult::success());
        }

        // Handle slurp mode - collect all inputs into array
        if opts.slurp {
            let values: Result<Vec<Val>, _> = jaq_json::read::parse_many(&input_bytes).collect();
            match values {
                Ok(vals) => {
                    let array: Val = vals.into_iter().collect();
                    return run_filter(&filter, array, &opts, &mut context);
                }
                Err(e) => {
                    writeln!(context.stderr(), "jq: parse error: {}", e)?;
                    return Ok(ExecutionResult::new(4));
                }
            }
        }

        // Normal mode: process each JSON value
        let mut last_code = 0;
        for result in jaq_json::read::parse_many(&input_bytes) {
            match result {
                Ok(val) => {
                    let exec_result = run_filter(&filter, val, &opts, &mut context)?;
                    if !exec_result.is_success() {
                        last_code = 1;
                    }
                }
                Err(e) => {
                    writeln!(context.stderr(), "jq: parse error: {}", e)?;
                    return Ok(ExecutionResult::new(4));
                }
            }
        }

        // Ensure output is flushed
        context.stdout().flush()?;

        Ok(ExecutionResult::new(last_code))
    }
}

fn run_filter<SE: ShellExtensions>(
    filter: &jaq_core::Filter<data::JustLut<Val>>,
    input: Val,
    opts: &JqOpts,
    context: &mut ExecutionContext<'_, SE>,
) -> Result<ExecutionResult, brush_core::Error> {
    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));

    let mut had_output = false;
    for result in filter.id.run((ctx.clone(), input)).map(unwrap_valr) {
        match result {
            Ok(val) => {
                had_output = true;
                output_value(&val, opts, context)?;
            }
            Err(e) => {
                writeln!(context.stderr(), "jq: {:?}", e)?;
                return Ok(ExecutionResult::new(5));
            }
        }
    }

    if !had_output && opts.exit_status {
        return Ok(ExecutionResult::new(1));
    }

    Ok(ExecutionResult::success())
}

fn output_value<SE: ShellExtensions>(
    val: &Val,
    opts: &JqOpts,
    context: &mut ExecutionContext<'_, SE>,
) -> Result<(), brush_core::Error> {
    // Check for raw string output
    if opts.raw_output
        && let Val::Str(s, _) = val
    {
        writeln!(context.stdout(), "{}", String::from_utf8_lossy(s.as_ref()))?;
        return Ok(());
    }

    // jaq_json Val implements Display
    if opts.compact {
        writeln!(context.stdout(), "{}", val)?;
    } else {
        // For pretty printing, convert to serde_json
        let json_str = format!("{}", val);
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
            writeln!(
                context.stdout(),
                "{}",
                serde_json::to_string_pretty(&parsed).unwrap_or(json_str)
            )?;
        } else {
            writeln!(context.stdout(), "{}", val)?;
        }
    }
    Ok(())
}

struct JqOpts {
    filter: String,
    files: Vec<String>,
    compact: bool,
    raw_output: bool,
    raw_input: bool,
    slurp: bool,
    null_input: bool,
    exit_status: bool,
}

impl JqOpts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut opts = JqOpts {
            filter: ".".to_string(),
            files: Vec::new(),
            compact: false,
            raw_output: false,
            raw_input: false,
            slurp: false,
            null_input: false,
            exit_status: false,
        };

        let mut positional = Vec::new();
        for arg in args.iter() {
            match arg.as_str() {
                "-c" | "--compact-output" => opts.compact = true,
                "-r" | "--raw-output" => opts.raw_output = true,
                "-R" | "--raw-input" => opts.raw_input = true,
                "-s" | "--slurp" => opts.slurp = true,
                "-n" | "--null-input" => opts.null_input = true,
                "-e" | "--exit-status" => opts.exit_status = true,
                s if s.starts_with('-') && s.len() > 1 && !s.starts_with("--") => {
                    // Handle combined short options like -cr
                    for c in s[1..].chars() {
                        match c {
                            'c' => opts.compact = true,
                            'r' => opts.raw_output = true,
                            'R' => opts.raw_input = true,
                            's' => opts.slurp = true,
                            'n' => opts.null_input = true,
                            'e' => opts.exit_status = true,
                            _ => return Err(format!("unknown option: -{}", c)),
                        }
                    }
                }
                _ => positional.push(arg.clone()),
            }
        }

        if !positional.is_empty() {
            opts.filter = positional.remove(0);
        }
        opts.files = positional;

        Ok(opts)
    }
}
