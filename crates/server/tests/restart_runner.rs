//! The public CLI checks one runner over its socket and stops only its named unit.
use boblm_store::Store;
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    sync::mpsc,
};

struct Fixture {
    temp: tempfile::TempDir,
    store: Store,
    config: PathBuf,
    socket: PathBuf,
    status: Arc<Mutex<Value>>,
    observations: mpsc::UnboundedReceiver<()>,
    host: tokio::task::JoinHandle<()>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("test.sqlite");
        let store = Store::open(&database).unwrap();
        store
            .create_session("wanted", temp.path().to_str().unwrap(), 1)
            .unwrap();
        store
            .create_session("untouched", temp.path().to_str().unwrap(), 2)
            .unwrap();
        let config = temp.path().join("boblm.json");
        std::fs::write(
            &config,
            json!({"token":"runner-restart-test-token-only", "database":database, "port":39996})
                .to_string(),
        )
        .unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        let systemctl = temp.path().join("systemctl");
        std::fs::write(
            &systemctl,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$RESTART_UNIT_LOG\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&systemctl, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = boblm_server::persistent::socket_path(&database, "wanted").unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let status = Arc::new(Mutex::new(json!({
            "jobs":{"available":true,"tasks":[]}, "usage":null,
            "stopped":false,"selection_pending":false,"queue_held":false,
            "running":true,"turn_active":false,"stopping":false,
            "queued":0,"dispatching":0,"uncertain":0,"rejected":0,
            "archived":false,"model":"sonnet","effort":null
        })));
        let current = status.clone();
        let (observed, observations) = mpsc::unbounded_channel();
        let host = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let length = socket.read_u32().await.unwrap();
                let mut bytes = vec![0; length as usize];
                socket.read_exact(&mut bytes).await.unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(request["session"], "wanted");
                assert!(
                    request["action"].is_null(),
                    "quiet checks must be read-only"
                );
                let status = current.lock().unwrap().clone();
                let response = serde_json::to_vec(&json!({
                    "version":boblm_tuning::RUNNER_PROTOCOL_VERSION,
                    "result":{"Ok":{"Status":status}}
                }))
                .unwrap();
                socket.write_u32(response.len() as u32).await.unwrap();
                socket.write_all(&response).await.unwrap();
                let _ = observed.send(());
            }
        });
        Self {
            temp,
            store,
            config,
            socket,
            status,
            observations,
            host,
        }
    }
    fn event(&self, kind: &str, age: Duration) {
        self.store
            .record_event(
                "wanted",
                "agent_event",
                json!({"type":kind}),
                chrono::Utc::now().timestamp_millis() - age.as_millis() as i64,
            )
            .unwrap();
    }
    fn command(&self, wait: &str) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_boblm"));
        command
            .args([
                "restart-runner",
                "--session",
                "wanted",
                "--wait",
                wait,
                "--config",
            ])
            .arg(&self.config)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.temp.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("RESTART_UNIT_LOG", self.temp.path().join("stops"))
            .kill_on_drop(true);
        command
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.abort();
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[tokio::test]
async fn restart_runner_command_waits_for_quiet_and_stops_only_that_unit() {
    let mut fixture = Fixture::new();
    fixture.event("assistant", boblm_tuning::RUNNER_QUIET_PERIOD);
    fixture.status.lock().unwrap()["turn_active"] = true.into();
    let mut command = fixture.command("5");
    let output = tokio::spawn(async move { command.output().await.unwrap() });
    tokio::time::timeout(Duration::from_secs(5), fixture.observations.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(!fixture.temp.path().join("stops").exists());
    fixture.event("result", boblm_tuning::RUNNER_QUIET_PERIOD);
    fixture.status.lock().unwrap()["turn_active"] = false.into();
    let output = tokio::time::timeout(Duration::from_secs(10), output)
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let key = fixture.socket.file_stem().unwrap().to_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.temp.path().join("stops")).unwrap(),
        format!("--user\nstop\nboblm-session-{key}.service\n")
    );
}

#[tokio::test]
async fn restart_runner_command_refuses_on_timeout_and_leaves_units_running() {
    let fixture = Fixture::new();
    fixture.event("result", Duration::ZERO);
    let output = fixture.command("1").output().await.unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("did not become quiet before the restart deadline"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.temp.path().join("stops").exists());
}

#[tokio::test]
async fn restart_runner_command_requires_a_result_and_known_empty_work() {
    for reason in [
        "assistant",
        "jobs",
        "unknown jobs",
        "turn",
        "queued",
        "dispatching",
        "uncertain",
        "durable queue",
    ] {
        let fixture = Fixture::new();
        fixture.event(
            if reason == "assistant" {
                "assistant"
            } else {
                "result"
            },
            boblm_tuning::RUNNER_QUIET_PERIOD,
        );
        {
            let mut status = fixture.status.lock().unwrap();
            match reason {
                "jobs" => {
                    status["jobs"]["tasks"] = json!([{"id":"job","command":"work","agentKey":null}])
                }
                "unknown jobs" => status["jobs"]["available"] = false.into(),
                "turn" => status["turn_active"] = true.into(),
                "queued" | "dispatching" | "uncertain" => status[reason] = 1.into(),
                _ => {}
            }
        }
        if reason == "durable queue" {
            fixture
                .store
                .append(boblm_store::NewMessage {
                    uuid: "00000000-0000-4000-8000-000000000001",
                    session: "wanted",
                    role: "user",
                    content: "new input behind the published snapshot",
                    delivery_state: "queued",
                    created_at: chrono::Utc::now().timestamp_millis(),
                })
                .unwrap();
        }
        let output = fixture.command("1").output().await.unwrap();
        assert!(!output.status.success(), "{reason} must prevent a restart");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("did not become quiet"),
            "{reason}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!fixture.temp.path().join("stops").exists(), "{reason}");
    }
}

#[tokio::test]
async fn restart_runner_command_ignores_operational_events_after_a_result() {
    let fixture = Fixture::new();
    fixture.event("result", boblm_tuning::RUNNER_QUIET_PERIOD);
    for kind in ["usage", "background_jobs"] {
        fixture.event(kind, Duration::ZERO);
    }
    fixture
        .store
        .record_event(
            "wanted",
            "state_summary",
            json!({}),
            chrono::Utc::now().timestamp_millis(),
        )
        .unwrap();
    let output = fixture.command("2").output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fixture.temp.path().join("stops").exists());
}

#[tokio::test]
async fn restart_runner_command_detects_turn_starts_after_a_result() {
    for event in [
        json!({"type":"system","subtype":"init"}),
        json!({"type":"system","subtype":"status","status":"requesting"}),
        json!({"type":"system","subtype":"task_started"}),
        json!({"type":"task_started"}),
        json!({"type":"system","subtype":"turn_started"}),
        json!({"type":"command_lifecycle","state":"started"}),
    ] {
        let fixture = Fixture::new();
        fixture.event("result", boblm_tuning::RUNNER_QUIET_PERIOD);
        fixture
            .store
            .record_event(
                "wanted",
                "agent_event",
                event.clone(),
                chrono::Utc::now().timestamp_millis(),
            )
            .unwrap();
        // Even an outdated status snapshot must not hide the newer turn start.
        let output = fixture.command("1").output().await.unwrap();
        assert!(!output.status.success(), "{event}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("did not become quiet"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!fixture.temp.path().join("stops").exists());
    }
}
