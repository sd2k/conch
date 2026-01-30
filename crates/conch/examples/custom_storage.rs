//! Shared VFS storage example.
//!
//! Demonstrates sharing VFS storage between multiple shells or with external code.
//! This pattern is useful when you need to:
//!
//! - Share data between multiple shell instances
//! - Access VFS data from outside the shell
//! - Implement pre/post-processing workflows
//!
//! Run with: cargo run -p conch --example custom_storage --features embedded-shell

use conch::{InMemoryStorage, ResourceLimits, Shell, VfsStorage};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a shared storage instance
    // Note: We use Arc<dyn VfsStorage> for sharing between shells
    let storage: Arc<dyn VfsStorage> = Arc::new(InMemoryStorage::new());

    // Create the scratch directory first
    storage.mkdir("/scratch").await?;

    // Pre-populate the storage from host code
    storage
        .write("/scratch/config.txt", b"mode=production")
        .await?;
    storage
        .write("/scratch/data.json", br#"{"items": [1, 2, 3]}"#)
        .await?;

    println!("=== Pre-populated storage ===");
    let files = storage.list("/scratch").await?;
    for entry in &files {
        println!("  - {}", entry.name);
    }

    // Create first shell using the shared storage
    let mut shell1 = Shell::builder()
        .vfs_arc(Arc::clone(&storage))
        .build()
        .await?;

    println!("\n=== Shell 1: Read and process data ===");
    let result = shell1
        .execute(
            "cat /scratch/data.json | jq '.items | length'",
            &ResourceLimits::default(),
        )
        .await?;
    println!(
        "Item count: {}",
        String::from_utf8_lossy(&result.stdout).trim()
    );

    // Shell 1 writes a result using file redirect
    let result = shell1
        .execute(
            "echo 'processed by shell1' > /scratch/result1.txt",
            &ResourceLimits::default(),
        )
        .await?;
    assert_eq!(result.exit_code, 0, "Shell 1 write failed");

    // Create second shell using the same storage - it sees shell1's output
    let mut shell2 = Shell::builder()
        .vfs_arc(Arc::clone(&storage))
        .build()
        .await?;

    println!("\n=== Shell 2: Sees Shell 1's output ===");
    let result = shell2
        .execute("cat /scratch/result1.txt", &ResourceLimits::default())
        .await?;
    println!(
        "From shell 2: {}",
        String::from_utf8_lossy(&result.stdout).trim()
    );

    // Shell 2 appends to the file
    let result = shell2
        .execute(
            "echo 'appended by shell2' >> /scratch/result1.txt",
            &ResourceLimits::default(),
        )
        .await?;
    assert_eq!(result.exit_code, 0, "Shell 2 append failed");

    // Host code can read the combined result
    println!("\n=== Host reads combined output ===");
    let content = storage.read("/scratch/result1.txt").await?;
    println!("File content:\n{}", String::from_utf8_lossy(&content));

    // Demonstrate processing workflow with redirects
    println!("=== Processing workflow with redirects ===");

    // Host writes input
    storage
        .write(
            "/scratch/input.csv",
            b"name,age,city\nalice,30,NYC\nbob,25,LA\ncharlie,35,CHI",
        )
        .await?;

    // Shell processes it and writes output
    let result = shell1
        .execute(
            "cat /scratch/input.csv | tail -n +2 | wc -l > /scratch/row_count.txt",
            &ResourceLimits::default(),
        )
        .await?;
    assert_eq!(result.exit_code, 0);

    // Host reads the result
    let count = storage.read("/scratch/row_count.txt").await?;
    println!(
        "Number of data rows: {}",
        String::from_utf8_lossy(&count).trim()
    );

    println!("\n=== Final storage state ===");
    let files = storage.list("/scratch").await?;
    println!("Files in /scratch:");
    for entry in &files {
        let content = storage.read(&format!("/scratch/{}", entry.name)).await?;
        println!(
            "  {} ({} bytes): {:?}",
            entry.name,
            content.len(),
            String::from_utf8_lossy(&content).trim()
        );
    }

    Ok(())
}
