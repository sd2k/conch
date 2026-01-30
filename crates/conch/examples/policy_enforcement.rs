//! Policy Enforcement Example
//!
//! Demonstrates how to configure and use filesystem access policies with
//! the AgentSandbox. Policies provide a security layer that restricts what
//! files and directories an agent can access, independent of what commands
//! the agent tries to run.
//!
//! Run with: cargo run -p conch --example policy_enforcement --features embedded-shell

use conch::ResourceLimits;
use conch::agent::AgentSandbox;
use conch::policy::{PolicyBuilder, PolicyDecision, agent_sandbox_policy};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Policy Enforcement Example ===\n");

    let limits = ResourceLimits::default();

    // === Example 1: Using the pre-built agent sandbox policy ===
    println!("--- Example 1: Pre-built Agent Sandbox Policy ---");
    println!("The agent_sandbox_policy() provides standard security defaults:\n");
    println!("  - Read access to /agent/** (agent metadata)");
    println!("  - Read access to /tools/** (tool definitions)");
    println!("  - Read access to /history/** (conversation history)");
    println!("  - Write access to /agent/scratch/** (temporary files)");
    println!("  - All other paths are denied by default\n");

    let mut sandbox = AgentSandbox::builder("agent-001")
        .name("Restricted Agent")
        .policy(agent_sandbox_policy())
        .build()
        .await?;

    // This succeeds - reading agent metadata is allowed
    let result = sandbox.execute("cat /agent/metadata.json", &limits).await?;
    println!("Reading /agent/metadata.json: OK");
    println!("  Exit code: {}", result.exit_code);

    // This succeeds - writing to scratch is allowed
    let result = sandbox
        .execute("echo 'test data' > /agent/scratch/test.txt", &limits)
        .await?;
    println!("Writing /agent/scratch/test.txt: OK");
    println!("  Exit code: {}", result.exit_code);

    // This fails - writing to /agent root is denied
    let result = sandbox
        .execute("echo 'hacked' > /agent/metadata.json", &limits)
        .await?;
    println!("Writing /agent/metadata.json: DENIED");
    println!(
        "  Error: {}",
        String::from_utf8_lossy(&result.stderr).trim()
    );

    // === Example 2: Custom policy with PolicyBuilder ===
    println!("\n--- Example 2: Custom Policy with PolicyBuilder ---");
    println!("PolicyBuilder::new() creates a deny-by-default policy.");
    println!("You then explicitly allow what you want:\n");

    // PolicyBuilder::new() is deny-by-default
    let custom_policy = PolicyBuilder::new().allow_read("/agent/**").build();

    let mut sandbox = AgentSandbox::builder("agent-002")
        .name("Read-Only Agent")
        .policy(custom_policy)
        .build()
        .await?;

    // This succeeds - reading is allowed
    let result = sandbox.execute("cat /agent/metadata.json", &limits).await?;
    println!("Reading /agent/metadata.json: OK");
    println!("  Exit code: {}", result.exit_code);

    // This fails - even scratch writes are denied with this policy
    let result = sandbox
        .execute("echo 'test' > /agent/scratch/test.txt", &limits)
        .await?;
    println!("Writing /agent/scratch/test.txt: DENIED");
    println!(
        "  Error: {}",
        String::from_utf8_lossy(&result.stderr).trim()
    );

    // === Example 3: Fine-grained access control ===
    println!("\n--- Example 3: Fine-Grained Access Control ---");
    println!("Creating a policy that allows different access to different paths:\n");

    let fine_grained_policy = PolicyBuilder::new()
        // Agent can read/write its own workspace
        .allow_write("/agent/scratch/**")
        .allow_write("/agent/state/**")
        // Agent can only read tool definitions
        .allow_read("/tools/**")
        // Agent can read its own metadata
        .allow_read("/agent/**")
        // Default is deny (PolicyBuilder::new() is deny-by-default)
        .build();

    let mut sandbox = AgentSandbox::builder("agent-003")
        .name("Fine-Grained Agent")
        .policy(fine_grained_policy)
        .build()
        .await?;

    // Test various operations
    let tests = [
        ("cat /agent/metadata.json", "Read agent metadata"),
        ("cat /tools/index.txt", "Read tools index"),
        ("echo 'data' > /agent/scratch/file.txt", "Write to scratch"),
        ("echo 'state' > /agent/state/data.json", "Write to state"),
        ("echo 'hack' > /agent/metadata.json", "Write to agent root"),
        ("cat /etc/passwd", "Read system file"),
    ];

    for (cmd, description) in tests {
        let result = sandbox.execute(cmd, &limits).await?;
        let status = if result.exit_code == 0 {
            "OK"
        } else {
            "DENIED"
        };
        println!("  {}: {}", description, status);
    }

    // === Example 4: Understanding PolicyDecision ===
    println!("\n--- Example 4: PolicyDecision Types ---");
    println!("Policies return one of two decisions:\n");
    println!("  {:?} - Operation is allowed", PolicyDecision::Allow);
    println!(
        "  {:?} - Operation is denied with a message",
        PolicyDecision::Deny("access denied by policy".to_string())
    );
    println!("\nThe denial message includes the operation and path for debugging.");

    // === Example 5: Allow-by-default policy ===
    println!("\n--- Example 5: Allow-by-Default Policy ---");
    println!("PolicyBuilder::allow_by_default() creates a permissive policy.");
    println!("You then explicitly deny what you don't want:\n");

    let permissive_policy = PolicyBuilder::allow_by_default()
        .deny_write("/agent/metadata.json") // Only deny writing to metadata
        .build();

    let mut sandbox = AgentSandbox::builder("agent-004")
        .name("Permissive Agent")
        .policy(permissive_policy)
        .build()
        .await?;

    // Most operations succeed because default is allow
    let result = sandbox.execute("cat /agent/metadata.json", &limits).await?;
    println!(
        "  Read /agent/metadata.json: exit_code={}",
        result.exit_code
    );

    let result = sandbox
        .execute("echo 'test' > /agent/scratch/test.txt", &limits)
        .await?;
    println!(
        "  Write /agent/scratch/test.txt: exit_code={}",
        result.exit_code
    );

    // But the specific deny rule still applies
    let result = sandbox
        .execute("echo 'hack' > /agent/metadata.json", &limits)
        .await?;
    println!(
        "  Write /agent/metadata.json: exit_code={} (denied by explicit rule)",
        result.exit_code
    );

    // === Example 6: Denying specific patterns ===
    println!("\n--- Example 6: Deny Specific Patterns ---");
    println!("Rules are evaluated in order - first match wins:\n");

    let selective_policy = PolicyBuilder::new()
        // Deny secrets first (evaluated before the allow rule)
        .deny_read("/agent/secrets/**")
        // Then allow the rest of /agent
        .allow_read("/agent/**")
        .allow_write("/agent/scratch/**")
        .build();

    let mut sandbox = AgentSandbox::builder("agent-005")
        .name("Selective Agent")
        .policy(selective_policy)
        .build()
        .await?;

    let result = sandbox.execute("cat /agent/metadata.json", &limits).await?;
    println!(
        "  Read /agent/metadata.json: exit_code={}",
        result.exit_code
    );

    // If there were secrets, this would be denied
    // (the file doesn't exist, but the policy would deny it anyway)
    println!("  Policy would deny: /agent/secrets/api_key.txt");

    println!("\n=== Example Complete ===");

    Ok(())
}
