//! Policy handler trait and implementations.

use std::sync::RwLock;

/// Information about the currently executing command.
///
/// This provides context for policy decisions, allowing rules like
/// "allow `grep` to read more paths than `cat`".
#[derive(Clone, Debug, Default)]
pub struct CommandInfo {
    /// The command name (e.g., "cat", "grep", "tool")
    pub name: String,
    /// Command arguments
    pub args: Vec<String>,
}

impl CommandInfo {
    /// Create a new command info.
    pub fn new(name: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}

/// The type of filesystem operation being performed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Reading file contents
    Read,
    /// Writing file contents
    Write,
    /// Deleting a file
    Delete,
    /// Listing directory contents
    List,
    /// Getting file/directory metadata
    Stat,
    /// Creating a directory
    Mkdir,
    /// Removing a directory
    Rmdir,
    /// Renaming/moving a file or directory
    Rename,
}

impl Operation {
    /// Returns true if this is a read-only operation.
    pub fn is_read_only(&self) -> bool {
        matches!(self, Operation::Read | Operation::List | Operation::Stat)
    }

    /// Returns true if this is a write operation.
    pub fn is_write(&self) -> bool {
        !self.is_read_only()
    }
}

/// The result of a policy check.
#[derive(Clone, Debug)]
pub enum PolicyDecision {
    /// The operation is allowed.
    Allow,
    /// The operation is denied with a reason.
    Deny(String),
}

impl PolicyDecision {
    /// Returns true if the operation is allowed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyDecision::Allow)
    }

    /// Returns the denial reason if denied, None if allowed.
    pub fn denial_reason(&self) -> Option<&str> {
        match self {
            PolicyDecision::Allow => None,
            PolicyDecision::Deny(reason) => Some(reason),
        }
    }
}

/// Trait for policy handlers that check filesystem access.
///
/// Implementations should be fast and non-blocking - policy checks happen
/// on every filesystem operation. For complex policies, consider caching
/// pattern matching results.
pub trait PolicyHandler: Send + Sync {
    /// Check if an operation is allowed.
    ///
    /// # Arguments
    ///
    /// * `path` - The path being accessed (absolute, normalized)
    /// * `operation` - The type of operation
    /// * `command` - The command performing the operation (if known)
    ///
    /// # Returns
    ///
    /// `PolicyDecision::Allow` if the operation is permitted,
    /// `PolicyDecision::Deny(reason)` if not.
    fn check_access(
        &self,
        path: &str,
        operation: Operation,
        command: Option<&CommandInfo>,
    ) -> PolicyDecision;
}

// Implement PolicyHandler for Arc<dyn PolicyHandler> to allow dynamic dispatch
impl PolicyHandler for std::sync::Arc<dyn PolicyHandler> {
    fn check_access(
        &self,
        path: &str,
        operation: Operation,
        command: Option<&CommandInfo>,
    ) -> PolicyDecision {
        (**self).check_access(path, operation, command)
    }
}

/// A policy that allows all operations.
///
/// This is the default policy, providing backward compatibility.
#[derive(Clone, Debug, Default)]
pub struct AllowAllPolicy;

impl PolicyHandler for AllowAllPolicy {
    fn check_access(
        &self,
        _path: &str,
        _operation: Operation,
        _command: Option<&CommandInfo>,
    ) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

/// A policy that denies all operations.
///
/// Useful as a base for building restrictive policies.
#[derive(Clone, Debug, Default)]
pub struct DenyAllPolicy;

impl PolicyHandler for DenyAllPolicy {
    fn check_access(
        &self,
        path: &str,
        operation: Operation,
        _command: Option<&CommandInfo>,
    ) -> PolicyDecision {
        PolicyDecision::Deny(format!(
            "{:?} access to {} denied by policy",
            operation, path
        ))
    }
}

/// A rule in a policy.
#[derive(Clone, Debug)]
struct PolicyRule {
    /// Glob pattern to match paths
    pattern: glob::Pattern,
    /// Operations this rule applies to (None = all operations)
    operations: Option<Vec<Operation>>,
    /// Whether this rule allows or denies
    allow: bool,
}

impl PolicyRule {
    fn matches(&self, path: &str, operation: Operation) -> bool {
        // Check if pattern matches
        if !self.pattern.matches(path) {
            return false;
        }

        // Check if operation matches (None means all operations)
        match &self.operations {
            Some(ops) => ops.contains(&operation),
            None => true,
        }
    }
}

/// A configurable policy built from rules.
///
/// Rules are evaluated in order - the first matching rule determines the decision.
/// If no rules match, the default decision is used (deny by default).
#[derive(Clone, Debug)]
pub struct Policy {
    rules: Vec<PolicyRule>,
    default_decision: PolicyDecision,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            default_decision: PolicyDecision::Deny("no matching policy rule".to_string()),
        }
    }
}

impl Policy {
    /// Create a new empty policy with deny-by-default.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a policy that allows everything (for backward compatibility).
    pub fn allow_all() -> Self {
        Self {
            rules: Vec::new(),
            default_decision: PolicyDecision::Allow,
        }
    }
}

impl PolicyHandler for Policy {
    fn check_access(
        &self,
        path: &str,
        operation: Operation,
        _command: Option<&CommandInfo>,
    ) -> PolicyDecision {
        // Evaluate rules in order
        for rule in &self.rules {
            if rule.matches(path, operation) {
                return if rule.allow {
                    PolicyDecision::Allow
                } else {
                    PolicyDecision::Deny(format!(
                        "{:?} access to {} denied by policy rule",
                        operation, path
                    ))
                };
            }
        }

        // No rule matched, use default
        self.default_decision.clone()
    }
}

/// Builder for creating policies with a fluent API.
///
/// # Example
///
/// ```rust,ignore
/// let policy = PolicyBuilder::new()
///     .allow_read("/agent/**")
///     .allow_read("/tools/**")
///     .allow_write("/agent/scratch/**")
///     .deny_write("/agent/params.json")  // Read-only params
///     .build();
/// ```
#[derive(Clone, Debug, Default)]
pub struct PolicyBuilder {
    rules: Vec<PolicyRule>,
    default_allow: bool,
}

impl PolicyBuilder {
    /// Create a new policy builder with deny-by-default.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            default_allow: false,
        }
    }

    /// Create a new policy builder with allow-by-default.
    ///
    /// Use this for backward compatibility or when you want to only
    /// specify what to deny.
    pub fn allow_by_default() -> Self {
        Self {
            rules: Vec::new(),
            default_allow: true,
        }
    }

    /// Allow read operations matching the given glob pattern.
    ///
    /// If the pattern ends with `/**`, we also add a rule for the base directory
    /// itself (e.g., `/agent/**` also allows listing `/agent`).
    pub fn allow_read(mut self, pattern: &str) -> Self {
        // If pattern ends with /**, also add the base directory
        if let Some(base) = pattern.strip_suffix("/**")
            && let Ok(p) = glob::Pattern::new(base)
        {
            self.rules.push(PolicyRule {
                pattern: p,
                operations: Some(vec![Operation::Read, Operation::List, Operation::Stat]),
                allow: true,
            });
        }

        if let Ok(p) = glob::Pattern::new(pattern) {
            self.rules.push(PolicyRule {
                pattern: p,
                operations: Some(vec![Operation::Read, Operation::List, Operation::Stat]),
                allow: true,
            });
        }
        self
    }

    /// Allow write operations matching the given glob pattern.
    ///
    /// If the pattern ends with `/**`, we also add a rule for the base directory
    /// itself (e.g., `/agent/scratch/**` also allows operations on `/agent/scratch`).
    pub fn allow_write(mut self, pattern: &str) -> Self {
        // If pattern ends with /**, also add the base directory
        if let Some(base) = pattern.strip_suffix("/**")
            && let Ok(p) = glob::Pattern::new(base)
        {
            self.rules.push(PolicyRule {
                pattern: p,
                operations: Some(vec![
                    Operation::Write,
                    Operation::Delete,
                    Operation::Mkdir,
                    Operation::Rmdir,
                    Operation::Rename,
                ]),
                allow: true,
            });
        }

        if let Ok(p) = glob::Pattern::new(pattern) {
            self.rules.push(PolicyRule {
                pattern: p,
                operations: Some(vec![
                    Operation::Write,
                    Operation::Delete,
                    Operation::Mkdir,
                    Operation::Rmdir,
                    Operation::Rename,
                ]),
                allow: true,
            });
        }
        self
    }

    /// Allow all operations matching the given glob pattern.
    pub fn allow_all(mut self, pattern: &str) -> Self {
        if let Ok(p) = glob::Pattern::new(pattern) {
            self.rules.push(PolicyRule {
                pattern: p,
                operations: None,
                allow: true,
            });
        }
        self
    }

    /// Deny read operations matching the given glob pattern.
    pub fn deny_read(mut self, pattern: &str) -> Self {
        if let Ok(p) = glob::Pattern::new(pattern) {
            self.rules.push(PolicyRule {
                pattern: p,
                operations: Some(vec![Operation::Read, Operation::List, Operation::Stat]),
                allow: false,
            });
        }
        self
    }

    /// Deny write operations matching the given glob pattern.
    pub fn deny_write(mut self, pattern: &str) -> Self {
        if let Ok(p) = glob::Pattern::new(pattern) {
            self.rules.push(PolicyRule {
                pattern: p,
                operations: Some(vec![
                    Operation::Write,
                    Operation::Delete,
                    Operation::Mkdir,
                    Operation::Rmdir,
                    Operation::Rename,
                ]),
                allow: false,
            });
        }
        self
    }

    /// Deny all operations matching the given glob pattern.
    pub fn deny_all(mut self, pattern: &str) -> Self {
        if let Ok(p) = glob::Pattern::new(pattern) {
            self.rules.push(PolicyRule {
                pattern: p,
                operations: None,
                allow: false,
            });
        }
        self
    }

    /// Build the policy.
    pub fn build(self) -> Policy {
        Policy {
            rules: self.rules,
            default_decision: if self.default_allow {
                PolicyDecision::Allow
            } else {
                PolicyDecision::Deny("no matching policy rule".to_string())
            },
        }
    }
}

/// A wrapper that tracks the current command for policy context.
///
/// This is used by the shell to provide command context to policy checks.
#[derive(Debug, Default)]
pub struct CommandTracker {
    current: RwLock<Option<CommandInfo>>,
}

impl CommandTracker {
    /// Create a new command tracker.
    pub fn new() -> Self {
        Self {
            current: RwLock::new(None),
        }
    }

    /// Set the current command.
    pub fn begin_command(&self, cmd: CommandInfo) {
        if let Ok(mut guard) = self.current.write() {
            *guard = Some(cmd);
        }
    }

    /// Clear the current command.
    pub fn end_command(&self) {
        if let Ok(mut guard) = self.current.write() {
            *guard = None;
        }
    }

    /// Get the current command (if any).
    pub fn current(&self) -> Option<CommandInfo> {
        self.current.read().ok().and_then(|g| g.clone())
    }
}

/// Standard policy for agent sandboxes.
///
/// This provides a reasonable default policy for agent execution:
/// - Read access to `/agent/**`, `/tools/**`, `/history/**`
/// - Write access to `/agent/scratch/**`
/// - No access to other paths
pub fn agent_sandbox_policy() -> Policy {
    PolicyBuilder::new()
        .allow_read("/agent/**")
        .allow_read("/tools/**")
        .allow_read("/history/**")
        .allow_write("/agent/scratch/**")
        .build()
}

/// Policy with full read access but restricted writes.
///
/// Useful for agents that need to read the entire VFS but only write to scratch.
pub fn read_only_policy() -> Policy {
    PolicyBuilder::new()
        .allow_read("/**")
        .allow_write("/agent/scratch/**")
        .build()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_all_policy() {
        let policy = AllowAllPolicy;
        assert!(
            policy
                .check_access("/any/path", Operation::Read, None)
                .is_allowed()
        );
        assert!(
            policy
                .check_access("/any/path", Operation::Write, None)
                .is_allowed()
        );
    }

    #[test]
    fn test_deny_all_policy() {
        let policy = DenyAllPolicy;
        assert!(
            !policy
                .check_access("/any/path", Operation::Read, None)
                .is_allowed()
        );
        assert!(
            !policy
                .check_access("/any/path", Operation::Write, None)
                .is_allowed()
        );
    }

    #[test]
    fn test_policy_builder_allow_read() {
        let policy = PolicyBuilder::new().allow_read("/agent/**").build();

        assert!(
            policy
                .check_access("/agent/params.json", Operation::Read, None)
                .is_allowed()
        );
        assert!(
            policy
                .check_access("/agent/subdir/file.txt", Operation::Read, None)
                .is_allowed()
        );
        assert!(
            !policy
                .check_access("/other/file.txt", Operation::Read, None)
                .is_allowed()
        );
        // Write should be denied
        assert!(
            !policy
                .check_access("/agent/params.json", Operation::Write, None)
                .is_allowed()
        );
    }

    #[test]
    fn test_policy_builder_allow_write() {
        let policy = PolicyBuilder::new()
            .allow_read("/agent/**")
            .allow_write("/agent/scratch/**")
            .build();

        // Can read anywhere in /agent
        assert!(
            policy
                .check_access("/agent/params.json", Operation::Read, None)
                .is_allowed()
        );
        // Can write only in /agent/scratch
        assert!(
            policy
                .check_access("/agent/scratch/output.txt", Operation::Write, None)
                .is_allowed()
        );
        assert!(
            !policy
                .check_access("/agent/params.json", Operation::Write, None)
                .is_allowed()
        );
    }

    #[test]
    fn test_policy_rule_order() {
        // Deny specific file, allow rest of directory
        let policy = PolicyBuilder::new()
            .deny_read("/agent/secrets.json")
            .allow_read("/agent/**")
            .build();

        assert!(
            !policy
                .check_access("/agent/secrets.json", Operation::Read, None)
                .is_allowed()
        );
        assert!(
            policy
                .check_access("/agent/params.json", Operation::Read, None)
                .is_allowed()
        );
    }

    #[test]
    fn test_agent_sandbox_policy() {
        let policy = agent_sandbox_policy();

        // Can read agent directories
        assert!(
            policy
                .check_access("/agent/params.json", Operation::Read, None)
                .is_allowed()
        );
        assert!(
            policy
                .check_access("/tools/index.json", Operation::Read, None)
                .is_allowed()
        );
        assert!(
            policy
                .check_access("/history/current/transcript.md", Operation::Read, None)
                .is_allowed()
        );

        // Can write to scratch
        assert!(
            policy
                .check_access("/agent/scratch/output.txt", Operation::Write, None)
                .is_allowed()
        );

        // Cannot write elsewhere
        assert!(
            !policy
                .check_access("/agent/params.json", Operation::Write, None)
                .is_allowed()
        );
        assert!(
            !policy
                .check_access("/tools/index.json", Operation::Write, None)
                .is_allowed()
        );

        // Cannot read other paths
        assert!(
            !policy
                .check_access("/etc/passwd", Operation::Read, None)
                .is_allowed()
        );
    }

    #[test]
    fn test_command_tracker() {
        let tracker = CommandTracker::new();

        assert!(tracker.current().is_none());

        tracker.begin_command(CommandInfo::new(
            "cat",
            vec!["/agent/params.json".to_string()],
        ));
        let cmd = tracker.current().unwrap();
        assert_eq!(cmd.name, "cat");
        assert_eq!(cmd.args, vec!["/agent/params.json"]);

        tracker.end_command();
        assert!(tracker.current().is_none());
    }

    #[test]
    fn test_operation_classification() {
        assert!(Operation::Read.is_read_only());
        assert!(Operation::List.is_read_only());
        assert!(Operation::Stat.is_read_only());

        assert!(Operation::Write.is_write());
        assert!(Operation::Delete.is_write());
        assert!(Operation::Mkdir.is_write());
    }
}
