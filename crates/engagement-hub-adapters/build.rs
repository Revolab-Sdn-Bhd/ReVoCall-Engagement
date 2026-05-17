fn main() -> Result<(), Box<dyn std::error::Error>> {
    // build.rs runs with the crate dir as CWD; navigate up to the workspace root
    // so that proto paths resolve correctly.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .expect("could not locate workspace root")
        .to_path_buf();

    let include_path = workspace_root.join("proto");
    let protos = [
        workspace_root.join("proto/revocall/registry/v1/registry.proto"),
        workspace_root.join("proto/revocall/journey/v1/journey_manager.proto"),
        workspace_root.join("proto/revocall/voice/v1/voice_manager.proto"),
    ];

    tonic_build::configure()
        .build_server(true)
        .compile_protos(&protos, &[include_path])?;
    Ok(())
}
