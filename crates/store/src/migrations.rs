use std::path::Path;
include!(concat!(env!("OUT_DIR"), "/migrations.rs"));

/// Marmot's public migration API takes a directory, so materialize embedded SQL
/// beside the owned database for the duration of migration, then remove it.
pub(crate) fn apply(conn: &rusqlite::Connection, database: &Path) -> crate::Result<()> {
    let parent = database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let directory = tempfile::Builder::new()
        .prefix(".boblm-migrations-")
        .tempdir_in(parent)?;
    for (name, sql) in MIGRATIONS {
        std::fs::write(directory.path().join(name), sql)?;
    }
    marmot::migrations::migrate_connection(conn, directory.path(), "schema_versions")?;
    Ok(())
}
