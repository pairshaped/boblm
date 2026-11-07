use std::{os::unix::fs::PermissionsExt, process::Command};

fn temporary_root() -> tempfile::TempDir {
    tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap()
}

fn command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_boblm"));
    command
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("PATH", "")
        .current_dir(root);
    command
}

#[test]
fn init_creates_private_config_with_random_token_and_safe_defaults() {
    let root = temporary_root();
    let output = command(root.path()).arg("init").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = root.path().join("config/boblm/boblm.json");
    let bytes = std::fs::read(&path).unwrap();
    let config: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let token = config["token"].as_str().unwrap();
    assert_eq!(token.len(), 43); // 32 bytes in unpadded base64url.
    assert!(
        token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        config["database"],
        root.path()
            .join("data/boblm/boblm.sqlite")
            .to_str()
            .unwrap()
    );
    assert_eq!(config["port"], 3000);
    assert!(config.get("webRoot").is_none());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.matches(token).count(), 1);
    assert!(stdout.contains("http://127.0.0.1:3000"));
    assert!(stdout.contains("127.0.0.1"));
    assert!(!root.path().join("data/boblm/boblm.sqlite").exists());

    let repeat = command(root.path()).arg("init").output().unwrap();
    assert!(!repeat.status.success());
    assert!(repeat.stdout.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let second = root.path().join("other.json");
    let output = command(root.path())
        .args(["init", "--config"])
        .arg(&second)
        .output()
        .unwrap();
    assert!(output.status.success());
    let other: serde_json::Value = serde_json::from_slice(&std::fs::read(second).unwrap()).unwrap();
    assert_ne!(config["token"], other["token"]);
}

#[test]
fn init_falls_back_to_home_and_never_follows_an_existing_symlink() {
    let root = temporary_root();
    let output = command(root.path())
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_DATA_HOME", "relative-directory-is-ignored")
        .arg("init")
        .output()
        .unwrap();
    assert!(output.status.success());
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.path().join(".config/boblm/boblm.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        config["database"],
        root.path()
            .join(".local/share/boblm/boblm.sqlite")
            .to_str()
            .unwrap()
    );
    let destination = root.path().join("not-created.json");
    let link = root.path().join("link.json");
    std::os::unix::fs::symlink(&destination, &link).unwrap();
    let result = command(root.path())
        .args(["init", "--config"])
        .arg(&link)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!destination.exists());
    assert!(link.is_symlink());
}

#[test]
fn serve_refuses_group_or_other_access_before_opening_database() {
    let root = temporary_root();
    let path = root.path().join("config.json");
    assert!(
        command(root.path())
            .args(["init", "--config"])
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
    for mode in [0o640, 0o604, 0o620, 0o601] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let output = command(root.path())
            .args(["serve", "--config"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("chmod 600"));
        assert!(!root.path().join("data").exists());
    }
}

#[test]
fn tailscale_url_requires_https_and_a_matching_root_proxy() {
    use boblm_server::config::tailscale_url;
    let mut status = serde_json::json!({
        "TCP":{"8443":{"HTTPS":true}},
        "Web":{"host.example.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}
    });
    assert_eq!(
        tailscale_url(&status, 3000).as_deref(),
        Some("https://host.example.ts.net:8443")
    );
    assert_eq!(tailscale_url(&status, 3001), None);
    status["TCP"]["8443"]["HTTPS"] = false.into();
    assert_eq!(tailscale_url(&status, 3000), None);
    assert_eq!(tailscale_url(&serde_json::json!({}), 3000), None);
}

#[tokio::test]
async fn serve_uses_private_config_and_embedded_assets_without_a_working_checkout() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let root = temporary_root();
    let path = root.path().join("config.json");
    assert!(
        command(root.path())
            .args(["init", "--config"])
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["port"] = 0.into();
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let mut process = tokio::process::Command::from(command(root.path()));
    process
        .args(["serve", "--config"])
        .arg(&path)
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = process.spawn().unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(
        boblm_tuning::SERVER_STARTUP_TEST_TIMEOUT,
        stderr.read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    let address = line
        .trim()
        .strip_prefix("boblm listening on http://")
        .expect("listener address");
    let mut responses = Vec::new();
    for path in ["/", "/app.js", "/style.css", "/favicon.svg", "/config.json"] {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_to_end(&mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
        responses.push(String::from_utf8(bytes).unwrap());
    }
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    for response in &responses[..4] {
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(!response.contains(config["token"].as_str().unwrap()));
    }
    assert!(responses[0].contains("<title>BobLM</title>"));
    assert!(responses[1].contains("text/javascript"));
    assert!(responses[2].contains("text/css"));
    assert!(responses[3].contains("image/svg+xml"));
    assert!(responses[3].contains("<svg"));
    assert!(responses[4].starts_with("HTTP/1.1 404"));
    let data = root.path().join("data/boblm");
    assert!(data.join("boblm.sqlite").exists());
    assert_eq!(
        std::fs::metadata(&data).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for entry in std::fs::read_dir(&data).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(
            entry.metadata().unwrap().permissions().mode() & 0o077,
            0,
            "{} must not be readable by group or others",
            entry.path().display()
        );
    }
}

#[test]
fn serve_refuses_an_existing_database_that_others_can_read() {
    let root = temporary_root();
    let path = root.path().join("config.json");
    assert!(
        command(root.path())
            .args(["init", "--config"])
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
    let data = root.path().join("data/boblm");
    std::fs::create_dir_all(&data).unwrap();
    let database = data.join("boblm.sqlite");
    std::fs::write(&database, b"").unwrap();
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o644)).unwrap();
    let output = command(root.path())
        .args(["serve", "--config"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("chmod 600"));
    assert_eq!(
        std::fs::metadata(&database).unwrap().permissions().mode() & 0o777,
        0o644,
        "refusal must not change or open the file"
    );
}

#[test]
fn relative_database_paths_become_absolute_when_the_config_is_read() {
    let root = temporary_root();
    let path = root.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"token":"0123456789abcdef0123","database":"rel/boblm.sqlite","port":3000}"#,
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let config = boblm_server::config::read(&path).unwrap();
    assert!(config.database.is_absolute(), "{:?}", config.database);
    assert!(config.database.ends_with("rel/boblm.sqlite"));
}

#[test]
fn serve_refuses_an_uploads_directory_that_others_can_use() {
    let root = temporary_root();
    let path = root.path().join("config.json");
    assert!(
        command(root.path())
            .args(["init", "--config"])
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
    let uploads = root.path().join("data/boblm/uploads");
    std::fs::create_dir_all(&uploads).unwrap();
    std::fs::set_permissions(&uploads, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = command(root.path())
        .args(["serve", "--config"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("chmod 700"));
}

#[test]
fn serve_refuses_a_wide_runtime_directory_before_deleting_anything_in_it() {
    let root = temporary_root();
    let path = root.path().join("config.json");
    assert!(
        command(root.path())
            .args(["init", "--config"])
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
    let runtime = root.path().join("runtime/boblm");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
    let stale = runtime.join("old.mcp.json");
    std::fs::write(&stale, b"{}").unwrap();
    let output = command(root.path())
        .env("XDG_RUNTIME_DIR", root.path().join("runtime"))
        .args(["serve", "--config"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("chmod 700"));
    assert!(
        stale.exists(),
        "nothing is deleted from a directory others can use"
    );
}
