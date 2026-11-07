//! First-run configuration and credential-file boundary.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::{
    fs::{DirBuilder, File},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

type Error = Box<dyn std::error::Error>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    pub token: String,
    pub database: PathBuf,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<crate::notifications::PushConfig>,
}

fn user_directory(variable: &str, fallback: &str) -> Result<PathBuf, Error> {
    if let Some(path) = std::env::var_os(variable).map(PathBuf::from)
        && path.is_absolute()
    {
        return Ok(path.join("boblm"));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("HOME must be an absolute path when XDG directories are unavailable")?;
    Ok(home.join(fallback).join("boblm"))
}

pub fn default_path() -> Result<PathBuf, Error> {
    Ok(user_directory("XDG_CONFIG_HOME", ".config")?.join("boblm.json"))
}

pub fn private_directory(path: &Path) -> io::Result<()> {
    if !path.as_os_str().is_empty() {
        DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    }
    Ok(())
}

/// Create a directory that only its owner may use, or refuse an existing one that
/// others can enter. Used for data that must not leak, such as uploads.
pub fn owner_only_directory(path: &Path) -> Result<(), Error> {
    private_directory(path)?;
    if std::fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "{} must not be accessible to group or others; run chmod 700 on it",
            path.display()
        )
        .into());
    }
    Ok(())
}

/// Where per-session agent files live: the user's runtime directory when there is
/// one, otherwise beside the database. Both are private to this user.
pub fn runtime_dir(database: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
        && path.is_absolute()
    {
        return path.join("boblm");
    }
    database.parent().unwrap_or(Path::new(".")).join("run")
}

/// Create the database's directory and an owner-only file before SQLite opens it;
/// SQLite gives its WAL and shared-memory files the main file's mode.
pub fn private_database(path: &Path) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        private_directory(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    // A database from an earlier run or another tool keeps its mode, so check it.
    if file.metadata()?.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "{} must not be accessible to group or others; run chmod 600 on it",
            path.display()
        )
        .into());
    }
    Ok(())
}

/// Publish a fully written config without replacing any existing directory entry.
pub fn init(path: &Path) -> Result<Config, Error> {
    let mut random = [0; 32];
    getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let config = Config {
        token: URL_SAFE_NO_PAD.encode(random),
        database: user_directory("XDG_DATA_HOME", ".local/share")?.join("boblm.sqlite"),
        port: boblm_tuning::DEFAULT_PORT,
        web_root: None,
        notifications: None,
    };
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    private_directory(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer_pretty(&mut file, &config)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|error| {
        io::Error::new(
            error.error.kind(),
            format!(
                "cannot create {}: {}; existing configs are never overwritten",
                path.display(),
                error.error
            ),
        )
    })?;
    Ok(config)
}

pub fn read(path: &Path) -> Result<Config, Error> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "{} must be a regular, owner-only config file; run chmod 600 on it",
            path.display()
        )
        .into());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut config: Config = serde_json::from_slice(&bytes)?;
    // Paths are relative to the working directory; agents receive absolute upload paths.
    config.database = std::path::absolute(&config.database)?;
    config.web_root = config.web_root.map(std::path::absolute).transpose()?;
    if config.token.len() < 16 || config.token.bytes().any(|b| b.is_ascii_whitespace()) {
        return Err("config token must contain at least 16 non-whitespace characters".into());
    }
    Ok(config)
}

/// Only advertise an existing root proxy that targets this listener.
pub fn tailscale_url(status: &serde_json::Value, port: u16) -> Option<String> {
    let targets = [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];
    status
        .get("Web")?
        .as_object()?
        .iter()
        .find_map(|(host, site)| {
            let target = site.get("Handlers")?.get("/")?.get("Proxy")?.as_str()?;
            let https_port = host.rsplit_once(':').map(|(_, p)| p).unwrap_or("443");
            let https = status
                .get("TCP")?
                .get(https_port)?
                .get("HTTPS")?
                .as_bool()?;
            (https
                && targets
                    .iter()
                    .any(|expected| target.trim_end_matches('/') == expected)
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:".contains(&b)))
            .then(|| format!("https://{host}"))
        })
}
