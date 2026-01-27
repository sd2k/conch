//! Custom builtins for conch-shell
//!
//! These builtins provide common Unix utilities that aren't available
//! as external commands in the WASM sandbox.

mod cat;
mod grep;
mod head;
mod jq;
mod tail;
mod tool;
mod wc;

pub use cat::CatCommand;
pub use grep::GrepCommand;
pub use head::HeadCommand;
pub use jq::JqCommand;
pub use tail::TailCommand;
#[allow(unused_imports)] // TOOL_REQUEST_EXIT_CODE used in Session 5
pub use tool::{TOOL_REQUEST_EXIT_CODE, ToolCommand};
pub use wc::WcCommand;

use std::collections::HashMap;

use brush_core::{builtins, extensions::ShellExtensions};

/// Register all conch builtins with the shell.
pub fn register_builtins<SE: ShellExtensions>(
    builtins: &mut HashMap<String, builtins::Registration<SE>>,
) {
    builtins.insert("cat".into(), builtins::simple_builtin::<CatCommand, SE>());
    builtins.insert("grep".into(), builtins::simple_builtin::<GrepCommand, SE>());
    builtins.insert("head".into(), builtins::simple_builtin::<HeadCommand, SE>());
    builtins.insert("jq".into(), builtins::simple_builtin::<JqCommand, SE>());
    builtins.insert("tail".into(), builtins::simple_builtin::<TailCommand, SE>());
    builtins.insert("tool".into(), builtins::simple_builtin::<ToolCommand, SE>());
    builtins.insert("wc".into(), builtins::simple_builtin::<WcCommand, SE>());
}
