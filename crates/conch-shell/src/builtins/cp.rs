//! cp builtin - copy files

use std::io::Write;

use brush_core::{ExecutionContext, ExecutionResult, ShellExtensions, builtins, error};

pub struct CpCommand;

impl builtins::SimpleCommand for CpCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, brush_core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => Ok("Copy files and directories.".into()),
            builtins::ContentType::ShortUsage => Ok("cp [-r] source dest".into()),
            builtins::ContentType::ShortDescription => Ok("cp - copy files and directories".into()),
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        context: ExecutionContext<'_, SE>,
        args: I,
    ) -> Result<ExecutionResult, brush_core::Error> {
        let mut recursive = false;
        let mut paths = Vec::new();
        let mut parsing_options = true;

        for arg in args.skip(1) {
            let arg = arg.as_ref();
            if parsing_options && arg == "--" {
                parsing_options = false;
            } else if parsing_options && (arg == "-r" || arg == "-R" || arg == "--recursive") {
                recursive = true;
            } else if parsing_options && arg.starts_with('-') && arg.len() > 1 {
                for c in arg[1..].chars() {
                    match c {
                        'r' | 'R' => recursive = true,
                        'f' | 'i' | 'n' | 'v' | 'a' | 'p' => {
                            // Common options we ignore for simplicity
                        }
                        _ => {
                            writeln!(context.stderr(), "cp: unknown option: -{}", c)?;
                            return Ok(ExecutionResult::new(1));
                        }
                    }
                }
            } else {
                paths.push(arg.to_string());
            }
        }

        if paths.len() < 2 {
            writeln!(context.stderr(), "cp: missing destination file operand")?;
            return Ok(ExecutionResult::new(1));
        }

        // Safe: we checked paths.len() >= 2 above
        let dest = paths.pop().expect("paths has at least 2 elements");
        let dest_path = std::path::Path::new(&dest);

        // If multiple sources, dest must be a directory
        if paths.len() > 1 && !dest_path.is_dir() {
            writeln!(context.stderr(), "cp: target '{}' is not a directory", dest)?;
            return Ok(ExecutionResult::new(1));
        }

        let mut exit_code = 0;

        for source in &paths {
            let src_path = std::path::Path::new(source);

            if !src_path.exists() {
                writeln!(
                    context.stderr(),
                    "cp: cannot stat '{}': No such file or directory",
                    source
                )?;
                exit_code = 1;
                continue;
            }

            let actual_dest = if dest_path.is_dir() {
                dest_path.join(src_path.file_name().unwrap_or_default())
            } else {
                dest_path.to_path_buf()
            };

            let result = if src_path.is_dir() {
                if recursive {
                    copy_dir_all(src_path, &actual_dest)
                } else {
                    writeln!(
                        context.stderr(),
                        "cp: -r not specified; omitting directory '{}'",
                        source
                    )?;
                    exit_code = 1;
                    continue;
                }
            } else {
                std::fs::copy(src_path, &actual_dest).map(|_| ())
            };

            if let Err(e) = result {
                writeln!(context.stderr(), "cp: cannot copy '{}': {}", source, e)?;
                exit_code = 1;
            }
        }

        Ok(ExecutionResult::new(exit_code))
    }
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}
