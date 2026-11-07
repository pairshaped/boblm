fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir_in(root.join("../../target"))?;
    let database = temp.path().join("schema.sqlite");
    marmot::migrations::migrate_from(&database, root.join("migrations"))?;
    let config = marmot::Config {
        database,
        source_root: root.join("src"),
        output: root.join("src/generated"),
        target: marmot::Target::Rust,
        check: std::env::args().any(|arg| arg == "--check"),
        temporal: Default::default(),
    };
    let project = marmot::analyze_project(&config)?;
    marmot::emit_project(&config, &project)?;
    Ok(())
}
