//! Optional strace wrapper that records who sends kill signals to Codex's processes (ADR 0050).
//! Off unless the server config names a directory. Any problem prints one warning and launches
//! Codex untraced, so the switch can never stop a chat from starting.
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::process::Command;

/// The command to launch `program` (the Codex binary), traced when `trace_dir` is set and usable.
/// The driver adds `app-server` and its other arguments afterwards.
pub fn command(program: &str, trace_dir: Option<&Path>, session_id: &str) -> Command {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    command_in(
        program,
        trace_dir,
        session_id,
        std::env::var_os("PATH").as_deref(),
        seconds,
    )
}

/// Like [`command`], with the PATH to search and the start time given, so tests need no environment.
pub fn command_in(
    program: &str,
    trace_dir: Option<&Path>,
    session_id: &str,
    path_var: Option<&OsStr>,
    seconds: u64,
) -> Command {
    let Some(dir) = trace_dir else {
        return Command::new(program);
    };
    match plan(dir, session_id, path_var, seconds) {
        Ok((strace, log)) => {
            let mut command = Command::new(strace);
            command
                .args(boblm_tuning::CODEX_KILL_TRACE_ARGS)
                .arg(boblm_tuning::CODEX_KILL_TRACE_OUTPUT_OPTION)
                .arg(log)
                .arg(program);
            command
        }
        Err(warning) => {
            eprintln!("{warning}");
            Command::new(program)
        }
    }
}

/// The strace executable and the log path, or the plain-English reason tracing is skipped.
fn plan(
    dir: &Path,
    session_id: &str,
    path_var: Option<&OsStr>,
    seconds: u64,
) -> Result<(PathBuf, PathBuf), String> {
    let skipped = "Codex will start without the kill trace.";
    if !dir.is_absolute() {
        return Err(format!(
            "The Codex kill trace directory {} is not an absolute path, so {skipped} Set codexKillTraceDir in the BobLM config to an absolute path, or remove it.",
            dir.display()
        ));
    }
    let program = boblm_tuning::CODEX_KILL_TRACE_PROGRAM;
    let Some(strace) = find_in_path(program, path_var) else {
        return Err(format!(
            "The Codex kill trace is on but {program} was not found on PATH, so {skipped} Install {program}, or remove codexKillTraceDir from the BobLM config."
        ));
    };
    if let Err(error) = create_private_dir(dir) {
        return Err(format!(
            "The Codex kill trace directory {} could not be created: {error}. {skipped} Check that its parent exists and is writable, or change codexKillTraceDir in the BobLM config.",
            dir.display()
        ));
    }
    let name = boblm_tuning::CODEX_KILL_TRACE_LOG_PATTERN
        .replace("{session}", session_id)
        .replace("{seconds}", &seconds.to_string());
    Ok((strace, dir.join(name)))
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// A cheap PATH lookup for an executable file; it never runs the program.
fn find_in_path(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path_var?)
        .map(|dir| dir.join(name))
        .find(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn shim_dir() -> tempfile::TempDir {
        let bin = tempfile::tempdir().unwrap();
        let path = bin.path().join(boblm_tuning::CODEX_KILL_TRACE_PROGRAM);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }
    fn args(command: &Command) -> Vec<String> {
        command
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_traced_launch_wraps_codex_in_strace_with_the_fixed_arguments() {
        let bin = shim_dir();
        let logs = tempfile::tempdir().unwrap();
        let dir = logs.path().join("kills");
        let command = command_in("codex", Some(&dir), "abc", Some(bin.path().as_os_str()), 77);
        assert_eq!(
            command.as_std().get_program(),
            bin.path().join("strace").as_os_str()
        );
        let mut want: Vec<String> = boblm_tuning::CODEX_KILL_TRACE_ARGS
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        want.push("-o".into());
        want.push(dir.join("codex-kills-abc-77.log").display().to_string());
        want.push("codex".into());
        assert_eq!(args(&command), want);
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn an_absent_directory_launches_codex_directly() {
        let bin = shim_dir();
        let command = command_in("codex", None, "abc", Some(bin.path().as_os_str()), 1);
        assert_eq!(command.as_std().get_program(), "codex");
        assert!(args(&command).is_empty());
    }

    #[test]
    fn a_relative_directory_falls_back_with_a_warning() {
        let bin = shim_dir();
        let warning = plan(
            Path::new("relative/dir"),
            "abc",
            Some(bin.path().as_os_str()),
            1,
        )
        .unwrap_err();
        assert!(warning.contains("not an absolute path"), "{warning}");
        let command = command_in(
            "codex",
            Some(Path::new("relative/dir")),
            "abc",
            Some(bin.path().as_os_str()),
            1,
        );
        assert_eq!(command.as_std().get_program(), "codex");
    }

    #[test]
    fn an_uncreatable_directory_falls_back_with_a_warning() {
        let bin = shim_dir();
        let file = tempfile::NamedTempFile::new().unwrap();
        let dir = file.path().join("under-a-file");
        let warning = plan(&dir, "abc", Some(bin.path().as_os_str()), 1).unwrap_err();
        assert!(warning.contains("could not be created"), "{warning}");
        let command = command_in("codex", Some(&dir), "abc", Some(bin.path().as_os_str()), 1);
        assert_eq!(command.as_std().get_program(), "codex");
    }

    #[test]
    fn a_missing_strace_falls_back_with_a_warning() {
        let empty = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        let warning = plan(logs.path(), "abc", Some(empty.path().as_os_str()), 1).unwrap_err();
        assert!(warning.contains("not found on PATH"), "{warning}");
        let command = command_in(
            "codex",
            Some(logs.path()),
            "abc",
            Some(empty.path().as_os_str()),
            1,
        );
        assert_eq!(command.as_std().get_program(), "codex");
        assert!(plan(logs.path(), "abc", None, 1).is_err());
    }
}
