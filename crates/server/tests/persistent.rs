//! Fake Codex in a real runner process; no model inference or production service.
use boblm_runner::RunningSession;
use boblm_store::{NewSession, Store};
use std::{os::unix::fs::PermissionsExt, time::Duration};

#[tokio::test]
async fn persistent_runner_process_keeps_agent_and_turn_across_client_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("runner.sqlite");
    boblm_server::config::private_database(&database).unwrap();
    let store = Store::open(&database).unwrap();
    store
        .create_session("bootstrap", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let project = store.reader().unwrap().sessions().unwrap()[0]
        .project_id
        .clone()
        .unwrap();
    store
        .create_session_in(NewSession {
            id: "runner",
            name: "runner",
            project_id: &project,
            agent: "codex",
            model: "gpt-6.1-sol",
            effort: Some("medium"),
            created_at: 2,
        })
        .unwrap();
    let config = temp.path().join("boblm.json");
    std::fs::write(
        &config,
        serde_json::json!({"token":"persistent-test-token-only","database":database,"port":39997})
            .to_string(),
    )
    .unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    let fake = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../codex-driver/tests/fake_app_server.py")
        .canonicalize()
        .unwrap();
    let executable = temp.path().join("codex");
    std::fs::write(
        &executable,
        format!("#!/bin/sh\nexec python3 '{}' \"$@\"\n", fake.display()),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let log = std::fs::File::create(temp.path().join("runner.log")).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_boblm"))
        .args([
            "session-runner",
            "--config",
            config.to_str().unwrap(),
            "--session",
            "runner",
        ])
        .env(
            "PATH",
            format!(
                "{}:{}",
                temp.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let socket = boblm_server::persistent::socket_path(&database, "runner").unwrap();
    let proxy = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(proxy) =
                RunningSession::attach(store.clone(), "runner", socket.clone(), None).await
            {
                break proxy;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let id = "f0000000-0000-4000-8000-000000000001";
    proxy.handle().send(id, "slow").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = proxy.handle().status().await.unwrap();
            if state.turn_active && state.dispatching == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let rejected = tokio::process::Command::new(env!("CARGO_BIN_EXE_boblm"))
        .args([
            "session-runner",
            "--config",
            config.to_str().unwrap(),
            "--session",
            "runner",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        !rejected.status.success(),
        "a second runner must not acquire the chat"
    );
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("already has a runner"));
    assert!(proxy.handle().status().await.unwrap().turn_active);
    let thread = store.reader().unwrap().agent_session_id("runner").unwrap();
    assert!(thread.is_some());
    proxy.shutdown().await.unwrap();
    assert!(child.try_wait().unwrap().is_none());
    let replacement = RunningSession::attach(store.clone(), "runner", socket.clone(), None)
        .await
        .unwrap();
    assert!(replacement.handle().status().await.unwrap().turn_active);
    assert_eq!(
        store.reader().unwrap().agent_session_id("runner").unwrap(),
        thread
    );
    assert!(!replacement.handle().send(id, "slow").await.unwrap());
    replacement.handle().stop().await.unwrap();
    assert!(replacement.handle().status().await.unwrap().stopped);
    replacement.shutdown().await.unwrap();
    let replacement = RunningSession::attach(store, "runner", socket.clone(), None)
        .await
        .unwrap();
    assert!(replacement.handle().status().await.unwrap().queue_held);
    replacement.shutdown().await.unwrap();
    assert!(
        tokio::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .await
            .unwrap()
            .success()
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert!(!socket.exists());
}
