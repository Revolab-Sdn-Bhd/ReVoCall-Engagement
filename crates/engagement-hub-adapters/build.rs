fn main() -> Result<(), Box<dyn std::error::Error>> {
    // build.rs runs with the crate dir as CWD; navigate up to the workspace root
    // so that proto/revocall/registry/v1/registry.proto resolves correctly.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .expect("could not locate workspace root")
        .to_path_buf();

    let proto_path = workspace_root.join("proto/revocall/registry/v1/registry.proto");
    let include_path = workspace_root.join("proto");

    tonic_build::configure()
        .build_server(true)
        .compile_protos(&[proto_path], &[include_path])?;
    Ok(())
}
