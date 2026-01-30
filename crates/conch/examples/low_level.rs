//! Low-level executor example.
//!
//! Demonstrates use of the Conch API for stateless execution with maximum control
//! over resource limits. This is useful when you need:
//!
//! - Custom resource limits per execution
//! - Stateless execution (no state persists between calls)
//! - Concurrency limiting for multiple scripts
//!
//! For stateful execution where variables persist, use the Shell API instead.
//!
//! Run with: cargo run -p conch --example low_level --features embedded-shell

use conch::{Conch, ResourceLimits};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create the Conch executor with embedded WASM bytes.
    // The second parameter is max_concurrent - how many scripts can run in parallel.
    let conch = Conch::embedded(4)?;

    println!("Conch executor created successfully");
    println!("Executor: {:?}\n", conch);

    // Execute with default limits
    println!("=== Basic execution ===");
    let result = conch
        .execute(
            "echo 'Hello from low-level API!'",
            ResourceLimits::default(),
        )
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));
    println!("exit code: {}", result.exit_code);

    // Execute with custom resource limits
    println!("\n=== Custom resource limits ===");
    let strict_limits = ResourceLimits {
        max_cpu_ms: 1000,                   // 1 second CPU time
        max_memory_bytes: 16 * 1024 * 1024, // 16 MB memory
        max_output_bytes: 4096,             // 4 KB output
        timeout: Duration::from_secs(5),    // 5 second wall clock
    };

    let result = conch
        .execute(
            "for i in 1 2 3; do echo \"iteration $i\"; done",
            strict_limits,
        )
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    // Execute multiple scripts - each gets isolated state (stateless execution)
    println!("\n=== Isolation between executions ===");

    // First execution sets a variable
    let result1 = conch
        .execute(
            "export MY_VAR=hello; echo $MY_VAR",
            ResourceLimits::default(),
        )
        .await?;
    println!("Execution 1: {}", String::from_utf8_lossy(&result1.stdout));

    // Second execution - variable should not exist (fresh instance each time)
    let result2 = conch
        .execute("echo \"MY_VAR is: '$MY_VAR'\"", ResourceLimits::default())
        .await?;
    println!("Execution 2: {}", String::from_utf8_lossy(&result2.stdout));

    // Demonstrate error handling
    println!("\n=== Error handling ===");
    let result = conch
        .execute("nonexistent_command", ResourceLimits::default())
        .await?;
    println!("exit code: {}", result.exit_code);
    println!("stderr: {}", String::from_utf8_lossy(&result.stderr));

    // Pipeline execution
    println!("\n=== Complex pipeline ===");
    let result = conch
        .execute(
            "echo -e 'apple\\nbanana\\napple\\ncherry\\napple' | sort | uniq -c | sort -rn",
            ResourceLimits::default(),
        )
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    Ok(())
}
