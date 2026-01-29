fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(true) // Useful for testing
        .compile_protos(&["proto/conch.proto"], &["proto/"])?;
    Ok(())
}
