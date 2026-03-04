//! Custom builtins for conch-shell
//!
//! These builtins provide common Unix utilities that aren't available
//! as external commands in the WASM sandbox.

mod cat;
mod cp;
mod grep;
mod head;
mod jq;
mod ls;
mod mkdir;
mod mv;
mod rm;
mod tail;
mod tool;
mod touch;
mod wc;

pub use cat::CatCommand;
pub use cp::CpCommand;
pub use grep::GrepCommand;
pub use head::HeadCommand;
pub use jq::JqCommand;
pub use ls::LsCommand;
pub use mkdir::MkdirCommand;
pub use mv::MvCommand;
pub use rm::RmCommand;
pub use tail::TailCommand;
pub use tool::ToolCommand;
pub use touch::TouchCommand;
pub use wc::WcCommand;

use std::collections::HashMap;

use brush_core::{builtins, extensions::ShellExtensions};

/// Register all conch builtins with the shell.
pub fn register_builtins<SE: ShellExtensions>(
    builtins: &mut HashMap<String, builtins::Registration<SE>>,
) {
    builtins.insert("cat".into(), builtins::simple_builtin::<CatCommand, SE>());
    builtins.insert("cp".into(), builtins::simple_builtin::<CpCommand, SE>());
    builtins.insert("grep".into(), builtins::simple_builtin::<GrepCommand, SE>());
    builtins.insert("head".into(), builtins::simple_builtin::<HeadCommand, SE>());
    builtins.insert("jq".into(), builtins::simple_builtin::<JqCommand, SE>());
    builtins.insert("ls".into(), builtins::simple_builtin::<LsCommand, SE>());
    builtins.insert("mkdir".into(), builtins::simple_builtin::<MkdirCommand, SE>());
    builtins.insert("mv".into(), builtins::simple_builtin::<MvCommand, SE>());
    builtins.insert("rm".into(), builtins::simple_builtin::<RmCommand, SE>());
    builtins.insert("tail".into(), builtins::simple_builtin::<TailCommand, SE>());
    builtins.insert("tool".into(), builtins::simple_builtin::<ToolCommand, SE>());
    builtins.insert("touch".into(), builtins::simple_builtin::<TouchCommand, SE>());
    builtins.insert("wc".into(), builtins::simple_builtin::<WcCommand, SE>());
}
