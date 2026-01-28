fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile the slipstream proto file
    tonic_build::configure()
        .build_server(false) // We only need the client
        .compile(&["proto/slipstream.proto"], &["proto"])?;
    Ok(())
}
