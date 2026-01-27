//! VFS and real filesystem mounts example.
//!
//! Demonstrates combining virtual storage with real filesystem access.
//!
//! Run with: cargo run -p conch --example vfs_mounts --features embedded-shell

use conch::{DirPerms, FilePerms, Mount, ResourceLimits, Shell};
use std::env;
use std::fs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a temporary directory for the example
    let temp_dir = env::temp_dir().join("conch-example");
    fs::create_dir_all(&temp_dir)?;

    // Create some files in the temp directory
    fs::write(temp_dir.join("readme.txt"), "This is a real file on disk.")?;
    fs::write(
        temp_dir.join("data.json"),
        r#"{"name": "test", "value": 42}"#,
    )?;

    println!("Created temp files in: {}", temp_dir.display());

    // Build a shell with both VFS and real filesystem mounts
    let shell = Shell::builder()
        // VFS paths (backed by in-memory storage)
        .vfs_path("/scratch", DirPerms::all(), FilePerms::all())
        .vfs_path("/config", DirPerms::READ, FilePerms::READ)
        // Real filesystem mount (read-only access to temp directory)
        .mount("/data", &temp_dir, Mount::readonly())
        .build()?;

    // Write some config to VFS
    shell
        .vfs()
        .write("/config/settings.txt", b"debug=true\nverbose=false")
        .await?;

    println!("\n=== List real filesystem mount ===");
    let result = shell
        .execute("ls -la /data", &ResourceLimits::default())
        .await?;
    println!("{}", String::from_utf8_lossy(&result.stdout));

    println!("=== Read real file ===");
    let result = shell
        .execute("cat /data/readme.txt", &ResourceLimits::default())
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    println!("=== Process JSON with jq ===");
    let result = shell
        .execute("cat /data/data.json | jq .name", &ResourceLimits::default())
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    println!("=== Read VFS config ===");
    let result = shell
        .execute("cat /config/settings.txt", &ResourceLimits::default())
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    println!("=== Combine data in pipeline ===");
    let result = shell
        .execute(
            "echo '=== README ===' && cat /data/readme.txt && echo '=== CONFIG ===' && cat /config/settings.txt",
            &ResourceLimits::default(),
        )
        .await?;
    println!("stdout: {}", String::from_utf8_lossy(&result.stdout));

    // Clean up
    fs::remove_dir_all(&temp_dir)?;
    println!("\nCleaned up temp directory.");

    Ok(())
}
