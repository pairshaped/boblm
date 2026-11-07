use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let migrations = root.join("migrations");
    println!("cargo:rerun-if-changed={}", migrations.display());
    let mut paths = fs::read_dir(&migrations)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().is_some_and(|extension| extension == "sql"));
    paths.sort();
    let mut generated = String::from("pub(crate) const MIGRATIONS: &[(&str, &str)] = &[\n");
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("non-UTF-8 migration name")?;
        generated.push_str(&format!("({name:?}, include_str!({path:?})),\n"));
    }
    generated.push_str("];\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("migrations.rs"),
        generated,
    )?;
    Ok(())
}
