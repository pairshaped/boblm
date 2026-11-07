//! Session processes supervised independently of the HTTP server.
use crate::{config, mcp::AgentFactory};
use boblm_runner::{Driver, DriverFactory, RunningSession, transport::RunnerLauncher};
use boblm_store::{Session, Store};
use sha2::{Digest, Sha256};
use std::{
    io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{net::UnixListener, process::Command};

fn runtime() -> io::Result<PathBuf> {
    let uid = std::fs::metadata("/proc/self")?.uid();
    Ok(PathBuf::from(format!("/run/user/{uid}")))
}
fn identity(database: &Path, session: &str) -> io::Result<String> {
    use std::os::unix::ffi::OsStrExt;
    let path = database.canonicalize()?;
    let namespace = format!("{:x}", Sha256::digest(path.as_os_str().as_bytes()));
    let mut digest = Sha256::new();
    digest.update(path.as_os_str().as_bytes());
    digest.update([0]);
    digest.update(session.as_bytes());
    Ok(format!(
        "{}-{}",
        &namespace[..16],
        &format!("{:x}", digest.finalize())[..32]
    ))
}
fn locations(database: &Path, session: &str) -> io::Result<(String, PathBuf)> {
    let key = identity(database, session)?;
    let directory = runtime()?.join("boblm-runners");
    config::owner_only_directory(&directory).map_err(|e| io::Error::other(e.to_string()))?;
    Ok((key.clone(), directory.join(format!("{key}.sock"))))
}
fn supervised(program: &str) -> io::Result<Command> {
    let directory = runtime()?;
    let mut command = Command::new(program);
    command.env("XDG_RUNTIME_DIR", &directory).env(
        "DBUS_SESSION_BUS_ADDRESS",
        format!("unix:path={}/bus", directory.display()),
    );
    Ok(command)
}

pub struct PersistentFactory {
    config: PathBuf,
}
impl PersistentFactory {
    pub fn new(config: PathBuf) -> io::Result<Self> {
        Ok(Self {
            config: config.canonicalize()?,
        })
    }
}
struct Launcher {
    config: PathBuf,
    database: PathBuf,
}
#[async_trait::async_trait]
impl RunnerLauncher for Launcher {
    async fn launch(&self, session: &str) -> io::Result<()> {
        launch(&self.config, &self.database, session).await
    }
}
impl PersistentFactory {
    fn launcher(&self, store: &Store) -> Arc<dyn RunnerLauncher> {
        Arc::new(Launcher {
            config: self.config.clone(),
            database: store.path().to_owned(),
        })
    }
}
/// Start the chat's runner unit unless it is already active.
async fn launch(config: &Path, database: &Path, session: &str) -> io::Result<()> {
    let (key, _) = locations(database, session)?;
    let unit = format!("boblm-session-{key}");
    let active = supervised("systemctl")?
        .args(["--user", "is-active", &unit])
        .output()
        .await?;
    if active.status.success() {
        return Ok(());
    }
    let mut launch = supervised("systemd-run")?;
    launch.args([
        "--user",
        "--collect",
        "--quiet",
        "--unit",
        &unit,
        "--property=KillMode=control-group",
        "--property=Restart=on-failure",
        "--property=StandardOutput=journal",
        "--property=StandardError=journal",
    ]);
    for name in ["PATH", "HOME", "CODEX_HOME", "CLAUDE_CONFIG_DIR"] {
        if let Ok(value) = std::env::var(name) {
            launch.arg(format!("--setenv={name}={value}"));
        }
    }
    let output = launch
        .arg(std::env::current_exe()?)
        .arg("session-runner")
        .arg("--config")
        .arg(config)
        .arg("--session")
        .arg(session)
        .output()
        .await?;
    // A concurrent attachment can have launched this same named unit. The
    // exclusive runner lock and successful handshake decide ownership.
    if !output.status.success() {
        eprintln!(
            "The session runner launch was not confirmed for {session}: {}. Checking its socket before reporting failure.",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
#[async_trait::async_trait]
impl DriverFactory for PersistentFactory {
    fn external_runners(&self) -> bool {
        true
    }
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        Err(io::Error::other(
            "An agent must be launched by its session runner. Reconnect the chat's runner before sending work.",
        ))
    }
    async fn connect_runner(
        &self,
        store: &Store,
        session: &str,
    ) -> boblm_runner::Result<Option<RunningSession>> {
        let (_, socket) = locations(store.path(), session)?;
        // A handshake is read-only. Never start a second actor behind an existing
        // runner whose protocol differs or whose command outcome is uncertain.
        match RunningSession::attach(
            store.clone(),
            session,
            socket.clone(),
            Some(self.launcher(store)),
        )
        .await
        {
            Ok(proxy) => return Ok(Some(proxy)),
            Err(boblm_runner::Error::Invalid(error)) => {
                eprintln!("Session {session} cannot attach to its runner: {error}");
                return RunningSession::attach_unavailable(
                    store.clone(),
                    session,
                    socket,
                    Some(self.launcher(store)),
                )
                .await
                .map(Some);
            }
            Err(_) => {}
        }
        launch(&self.config, store.path(), session).await?;
        let deadline = tokio::time::Instant::now() + boblm_tuning::RUNNER_CONNECT_TIMEOUT;
        loop {
            match RunningSession::attach(
                store.clone(),
                session,
                socket.clone(),
                Some(self.launcher(store)),
            )
            .await
            {
                Ok(proxy) => return Ok(Some(proxy)),
                Err(boblm_runner::Error::Invalid(error)) => {
                    eprintln!("Session {session} cannot attach to its runner: {error}");
                    return RunningSession::attach_unavailable(
                        store.clone(),
                        session,
                        socket,
                        Some(self.launcher(store)),
                    )
                    .await
                    .map(Some);
                }
                Err(error) if tokio::time::Instant::now() >= deadline => {
                    eprintln!("Session {session} could not attach to its runner: {error}");
                    return RunningSession::attach_unavailable(
                        store.clone(),
                        session,
                        socket,
                        Some(self.launcher(store)),
                    )
                    .await
                    .map(Some);
                }
                Err(_) => tokio::time::sleep(boblm_tuning::RUNNER_POLL_INTERVAL).await,
            }
        }
    }
}

/// Only the session unit owns the provider pipes and process groups. A held file
/// lock proves ownership before stale socket cleanup or any delivery-state recovery.
pub async fn serve(path: &Path, session: &str) -> Result<(), Box<dyn std::error::Error>> {
    let settings = config::read(path)?;
    if settings.port == 0 {
        return Err("Persistent runners require a fixed server port. Set the port in the instance configuration.".into());
    }
    config::private_database(&settings.database)?;
    let database = settings.database.canonicalize()?;
    let (key, socket) = locations(&database, session)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(socket.with_extension("lock"))?;
    lock.try_lock().map_err(
        |_| "This chat already has a runner. Reconnect to that runner instead of starting another.",
    )?;
    if socket.exists() {
        std::fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let store = tokio::task::spawn_blocking(move || Store::open(database)).await??;
    let lookup = store.clone();
    let wanted = session.to_owned();
    let session_info = tokio::task::spawn_blocking(move || {
        lookup
            .reader()?
            .sessions()
            .map(|rows| rows.into_iter().find(|s| s.id == wanted))
    })
    .await??;
    let Some(session_info) = session_info else {
        std::fs::remove_file(&socket)?;
        return Ok(());
    };
    let runtime_dir = socket
        .parent()
        .ok_or("The runner socket has no parent directory.")?
        .join(key);
    config::owner_only_directory(&runtime_dir)?;
    let factory = Arc::new(AgentFactory::new(
        format!("http://127.0.0.1:{}", settings.port),
        settings.token,
        runtime_dir,
        crate::uploads::uploads_dir(store.path()),
    ));
    let mut runner = RunningSession::start(store.clone(), session, factory).await?;
    let wake = if session_info.agent == "codex" {
        let handle = runner.handle();
        let session = session.to_owned();
        Some(tokio::spawn(async move {
            let mut observer = Some(crate::subagent_wake::Observer::new());
            let mut tick = tokio::time::interval(boblm_tuning::SCHEDULER_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let Ok(status) = handle.published_status() else {
                    break;
                };
                if !status.running || status.archived {
                    continue;
                }
                let source = store.clone();
                let id = session.clone();
                if let Ok(Ok(Some(parent))) =
                    tokio::task::spawn_blocking(move || source.reader()?.agent_session_id(&id))
                        .await
                {
                    crate::subagent_wake::wake_parent(&handle, parent, &mut observer).await;
                }
            }
        }))
    } else {
        None
    };
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let outcome = tokio::select! {
        result = boblm_runner::transport::serve(listener, runner.handle()) => {
            result?;
            runner.wait().await
        },
        _ = terminate.recv() => Ok(()),
        _ = tokio::signal::ctrl_c() => Ok(()),
    };
    if let Some(wake) = wake {
        wake.abort();
    }
    runner.shutdown().await?;
    std::fs::remove_file(&socket)?;
    drop(lock);
    outcome?;
    Ok(())
}

/// Explicit shutdown for an instance's runners, used by disposable dev instances.
/// Unit names are scoped to the canonical database path, never a PID or a global kill.
pub async fn stop_all(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::ffi::OsStrExt;
    let settings = config::read(path)?;
    let database = settings.database.canonicalize()?;
    let hash = format!("{:x}", Sha256::digest(database.as_os_str().as_bytes()));
    let prefix = format!("boblm-session-{}-", &hash[..16]);
    let units = supervised("systemctl")?
        .args([
            "--user",
            "list-units",
            "--all",
            "--no-legend",
            "--plain",
            "--type=service",
            &format!("{prefix}*"),
        ])
        .output()
        .await?;
    if !units.status.success() {
        return Err("The instance's runner units could not be listed. Check the user systemd service before retrying.".into());
    }
    for line in String::from_utf8(units.stdout)?.lines() {
        if let Some(unit) = line.split_whitespace().next()
            && unit.starts_with(&prefix)
            && unit.ends_with(".service")
        {
            let status = supervised("systemctl")?
                .args(["--user", "stop", unit])
                .status()
                .await?;
            if !status.success() {
                return Err(format!(
                    "Runner {unit} could not be stopped. Inspect that unit before retrying."
                )
                .into());
            }
        }
    }
    Ok(())
}

/// Diagnostic attachment path, scoped to this instance and chat.
pub fn socket_path(database: &Path, session: &str) -> io::Result<PathBuf> {
    locations(database, session).map(|(_, socket)| socket)
}

/// Wait for this chat alone to become quiet, then stop its supervised runner. The
/// web proxy relaunches the unit with the installed executable on its next poll.
pub async fn restart_runner(
    path: &Path,
    session: &str,
    wait: std::time::Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = config::read(path)?;
    let (key, socket) = locations(&settings.database, session)?;
    let database = settings.database;
    let store = tokio::task::spawn_blocking(move || Store::open(database)).await??;
    wait_for_quiet(&store, session, &socket, wait).await?;
    let unit = format!("boblm-session-{key}.service");
    let output = supervised("systemctl")?
        .args(["--user", "stop", &unit])
        .output()
        .await?;
    if !output.status.success() {
        return Err(format!(
            "This chat's runner could not be stopped: {}. Inspect unit {unit} before retrying.",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(())
}

async fn wait_for_quiet(
    store: &Store,
    session: &str,
    socket: &Path,
    wait: std::time::Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now()
        .checked_add(wait)
        .ok_or("The runner restart wait is too long. Choose a smaller number of seconds.")?;
    loop {
        let check = async {
            let status = boblm_runner::transport::status(socket, session).await?;
            let source = store.clone();
            let id = session.to_owned();
            let (event, messages) = tokio::task::spawn_blocking(move || {
                let reader = source.reader()?;
                Ok::<_, boblm_store::Error>((
                    reader.runner_restart_activity(&id)?,
                    reader.outstanding_messages(&id)?,
                ))
            })
            .await??;
            let silence = event.as_ref().is_some_and(|(payload, created_at)| {
                let result = serde_json::from_str::<serde_json::Value>(payload)
                    .is_ok_and(|event| event["type"] == "result");
                let age = chrono::Utc::now()
                    .timestamp_millis()
                    .saturating_sub(*created_at);
                result && age >= boblm_tuning::RUNNER_QUIET_PERIOD.as_millis() as i64
            });
            Ok::<_, Box<dyn std::error::Error>>(
                silence
                    && !status.turn_active
                    && !status.stopping
                    && status.queued == 0
                    && status.dispatching == 0
                    && status.uncertain == 0
                    && status.jobs.as_ref().map_or(!status.running, |jobs| {
                        jobs.available && jobs.tasks.is_empty()
                    })
                    && !messages.iter().any(|m| {
                        matches!(
                            m.delivery_state.as_str(),
                            "queued" | "dispatching" | "uncertain"
                        )
                    }),
            )
        };
        match tokio::time::timeout_at(deadline, check).await {
            Ok(Ok(true)) => return Ok(()),
            Ok(Err(error)) => return Err(error),
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("This chat did not become quiet before the restart deadline, so its runner was left running. Let its work finish and try again.".into());
        }
        tokio::time::sleep_until(
            deadline.min(tokio::time::Instant::now() + boblm_tuning::RUNNER_POLL_INTERVAL),
        )
        .await;
    }
}
