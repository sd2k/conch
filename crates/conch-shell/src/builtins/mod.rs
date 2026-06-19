//! Custom builtins for conch-shell
//!
//! conch-shell always ships a few non-coreutils builtins (`grep`, `jq`,
//! `tool`). The coreutils (cat, head, tail, ls, wc, cp, mv, rm, mkdir, touch,
//! …) are normally provided by spawning the uutils `coreutils` component (built
//! via `clis/coreutils.toml`, registered under each util name) — a single
//! battle-tested implementation rather than these hand-rolled ones. See #86.
//!
//! Spawning needs a host runtime (the `conch:shell/process` import), which the
//! browser/Node "lite" build doesn't have. So the hand-rolled coreutils
//! builtins are compiled in only when the `subprocess` feature is **off** —
//! i.e. for the lite build, where they're the only way `cat`/`ls`/… work. They
//! are acknowledged PoC-quality stopgaps; the real fix is a jco spawn shim that
//! runs the same uutils component in the browser (tracked separately).

mod grep;
mod jq;
mod tool;

pub use grep::GrepCommand;
pub use jq::JqCommand;
pub use tool::ToolCommand;

// Hand-rolled coreutils: lite build only (no subprocess spawning available).
#[cfg(not(feature = "subprocess"))]
mod cat;
#[cfg(not(feature = "subprocess"))]
mod cp;
#[cfg(not(feature = "subprocess"))]
mod head;
#[cfg(not(feature = "subprocess"))]
mod ls;
#[cfg(not(feature = "subprocess"))]
mod mkdir;
#[cfg(not(feature = "subprocess"))]
mod mv;
#[cfg(not(feature = "subprocess"))]
mod rm;
#[cfg(not(feature = "subprocess"))]
mod tail;
#[cfg(not(feature = "subprocess"))]
mod touch;
#[cfg(not(feature = "subprocess"))]
mod wc;

#[cfg(not(feature = "subprocess"))]
pub use cat::CatCommand;
#[cfg(not(feature = "subprocess"))]
pub use cp::CpCommand;
#[cfg(not(feature = "subprocess"))]
pub use head::HeadCommand;
#[cfg(not(feature = "subprocess"))]
pub use ls::LsCommand;
#[cfg(not(feature = "subprocess"))]
pub use mkdir::MkdirCommand;
#[cfg(not(feature = "subprocess"))]
pub use mv::MvCommand;
#[cfg(not(feature = "subprocess"))]
pub use rm::RmCommand;
#[cfg(not(feature = "subprocess"))]
pub use tail::TailCommand;
#[cfg(not(feature = "subprocess"))]
pub use touch::TouchCommand;
#[cfg(not(feature = "subprocess"))]
pub use wc::WcCommand;

use std::collections::HashMap;

use brush_core::{builtins, extensions::ShellExtensions};

/// Register all conch builtins with the shell.
pub fn register_builtins<SE: ShellExtensions>(
    builtins: &mut HashMap<String, builtins::Registration<SE>>,
) {
    builtins.insert("grep".into(), builtins::simple_builtin::<GrepCommand, SE>());
    builtins.insert("jq".into(), builtins::simple_builtin::<JqCommand, SE>());
    builtins.insert("tool".into(), builtins::simple_builtin::<ToolCommand, SE>());

    // Lite build only: spawned uutils coreutils replace these when the
    // `subprocess` feature is enabled (host builds).
    #[cfg(not(feature = "subprocess"))]
    {
        builtins.insert("cat".into(), builtins::simple_builtin::<CatCommand, SE>());
        builtins.insert("cp".into(), builtins::simple_builtin::<CpCommand, SE>());
        builtins.insert("head".into(), builtins::simple_builtin::<HeadCommand, SE>());
        builtins.insert("ls".into(), builtins::simple_builtin::<LsCommand, SE>());
        builtins.insert(
            "mkdir".into(),
            builtins::simple_builtin::<MkdirCommand, SE>(),
        );
        builtins.insert("mv".into(), builtins::simple_builtin::<MvCommand, SE>());
        builtins.insert("rm".into(), builtins::simple_builtin::<RmCommand, SE>());
        builtins.insert("tail".into(), builtins::simple_builtin::<TailCommand, SE>());
        builtins.insert(
            "touch".into(),
            builtins::simple_builtin::<TouchCommand, SE>(),
        );
        builtins.insert("wc".into(), builtins::simple_builtin::<WcCommand, SE>());
    }
}
