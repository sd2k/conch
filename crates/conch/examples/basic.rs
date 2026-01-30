//! Basic Shell API usage example.
//!
//! Demonstrates the high-level Shell API with VFS storage.
//!
//! Run with: cargo run -p conch --example basic --features embedded-shell

use conch::{ResourceLimits, Shell};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a shell with default settings.
    // This provides a /scratch directory backed by in-memory VFS storage.
    let mut shell = Shell::builder().build().await?;

    // Write some data to the VFS from the host
    shell
        .vfs()
        .write("/scratch/input.txt", b"hello world\nfoo bar\nbaz qux")
        .await?;

    println!("=== Basic execution ===");
    let result = shell
        .execute("echo 'Hello from conch!'", &ResourceLimits::default())
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));
    println!("exit code: {}\n", result.exit_code);

    println!("=== Reading VFS file ===");
    let result = shell
        .execute("cat /scratch/input.txt", &ResourceLimits::default())
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    println!("=== Pipeline with grep ===");
    let result = shell
        .execute(
            "cat /scratch/input.txt | grep foo",
            &ResourceLimits::default(),
        )
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    println!("=== Word count ===");
    let result = shell
        .execute("wc -l /scratch/input.txt", &ResourceLimits::default())
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    println!("=== Variables and arithmetic ===");
    let result = shell
        .execute(
            "x=5; y=3; echo \"Sum: $((x + y))\"",
            &ResourceLimits::default(),
        )
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    Ok(())
}
