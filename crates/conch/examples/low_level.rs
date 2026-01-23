//! Low-level executor example.
//!
//! Demonstrates direct use of ComponentShellExecutor for maximum control
//! over the WASM runtime. This is useful when you need:
//!
//! - Custom resource limits per execution
//! - Direct access to the wasmtime Engine
//! - Integration with existing WASM infrastructure
//!
//! Run with: cargo run -p conch --example low_level --features embedded-shell

use conch::{ComponentShellExecutor, ResourceLimits};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create the executor directly from embedded WASM bytes.
    // This gives you access to the underlying wasmtime Engine.
    let executor = ComponentShellExecutor::embedded()?;

    println!("Engine created successfully");
    println!("Engine reference: {:?}\n", executor.engine());

    // Execute with default limits
    println!("=== Basic execution ===");
    let result = executor
        .execute(
            "echo 'Hello from low-level API!'",
            &ResourceLimits::default(),
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

    let result = executor
        .execute(
            "for i in 1 2 3; do echo \"iteration $i\"; done",
            &strict_limits,
        )
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    // Execute multiple scripts - each gets isolated state
    println!("\n=== Isolation between executions ===");

    // First execution sets a variable
    let result1 = executor
        .execute(
            "export MY_VAR=hello; echo $MY_VAR",
            &ResourceLimits::default(),
        )
        .await?;
    println!("Execution 1: {}", String::from_utf8_lossy(&result1.stdout));

    // Second execution - variable should not exist
    let result2 = executor
        .execute("echo \"MY_VAR is: '$MY_VAR'\"", &ResourceLimits::default())
        .await?;
    println!("Execution 2: {}", String::from_utf8_lossy(&result2.stdout));

    // Clone the executor - both share the same Engine and InstancePre
    println!("\n=== Cloned executor ===");
    let executor2 = executor.clone();

    let result = executor2
        .execute("echo 'From cloned executor'", &ResourceLimits::default())
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    // Demonstrate error handling
    println!("\n=== Error handling ===");
    let result = executor
        .execute("nonexistent_command", &ResourceLimits::default())
        .await?;
    println!("exit code: {}", result.exit_code);
    println!("stderr: {}", String::from_utf8_lossy(&result.stderr));

    // Pipeline execution
    println!("\n=== Complex pipeline ===");
    let result = executor
        .execute(
            "echo -e 'apple\\nbanana\\napple\\ncherry\\napple' | sort | uniq -c | sort -rn",
            &ResourceLimits::default(),
        )
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    Ok(())
}
