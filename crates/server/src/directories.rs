//! Folder selection for projects. File contents are never opened here.
use boblm_protocol::client::{DirectoryInfo, DirectoryListing, shown};
use std::{fs, path::PathBuf};

pub(crate) fn list(requested: Option<String>) -> Result<DirectoryListing, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("The server has no absolute HOME directory. Set HOME for the boblm service before browsing folders.")?;
    let home = fs::canonicalize(home).map_err(|error| format!("The home folder could not be opened: {error}. Check HOME and the folder permissions for the boblm service."))?;
    let requested = requested.map(PathBuf::from).unwrap_or_else(|| home.clone());
    let failed = |error| {
        format!(
            "The folder {} could not be opened: {error}. Choose another folder or check its permissions.",
            shown(&requested.to_string_lossy())
        )
    };
    let path = fs::canonicalize(&requested).map_err(failed)?;
    if !path.starts_with(&home) {
        return Err("That folder is outside your home directory. Choose a folder inside your home directory.".into());
    }
    let entries = fs::read_dir(&path).map_err(failed)?;
    let mut directories = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= boblm_tuning::MAX_DIRECTORY_ENTRIES {
            return Err(format!(
                "This folder has more than {} entries, so it cannot be listed here. Choose another folder or reduce the number of entries and try again.",
                boblm_tuning::MAX_DIRECTORY_ENTRIES
            ));
        }
        let entry = entry.map_err(failed)?;
        if entry.file_name().as_encoded_bytes().starts_with(b".") {
            continue;
        }
        // Broken links, disappearing entries and links outside home are not browse targets.
        let Ok(resolved) = fs::canonicalize(entry.path()) else {
            continue;
        };
        if !resolved.starts_with(&home) || !resolved.is_dir() {
            continue;
        }
        let (Some(name), Some(path)) = (
            entry.file_name().to_str().map(str::to_owned),
            resolved.to_str(),
        ) else {
            continue;
        };
        if path.chars().count() <= boblm_tuning::MAX_WORKING_DIRECTORY_CHARS as usize {
            directories.push(DirectoryInfo {
                name,
                path: path.to_owned(),
            });
        }
    }
    directories.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.name.cmp(&b.name))
    });
    let listing = DirectoryListing {
        parent: (path != home)
            .then(|| path.parent().map(|p| p.to_string_lossy().into_owned()))
            .flatten(),
        path: path
            .to_str()
            .ok_or(
                "This folder's name is not valid UTF-8. Rename it before choosing it as a project.",
            )?
            .to_owned(),
        directories,
    };
    if serde_json::to_vec(&listing).map_err(|error| format!("The folder list could not be encoded: {error}. Choose another folder or reduce the number of entries and try again."))?.len() > boblm_tuning::MAX_BODY_BYTES {
        return Err("This folder list is too large to display. Choose another folder or reduce the number of entries and try again.".into());
    }
    Ok(listing)
}
