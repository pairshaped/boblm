//! Disposable SOPS identities and fictional provider keys; no home credential is read.
use std::{
    io::Write,
    process::{Command, Stdio},
};
#[tokio::test]
async fn pi_credentials_extract_one_key_and_report_value_free_failures() {
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let identity = dir.path().join("age.txt");
    let keygen = Command::new("age-keygen")
        .arg("-o")
        .arg(&identity)
        .output()
        .unwrap();
    assert!(keygen.status.success());
    let public = String::from_utf8(keygen.stderr)
        .unwrap()
        .split_whitespace()
        .find(|s| s.starts_with("age1"))
        .unwrap()
        .to_owned();
    // SAFETY: this test binary has a single test and no other SOPS commands running.
    unsafe {
        std::env::set_var("SOPS_AGE_KEY_FILE", &identity);
    }
    let file = dir.path().join("keys.json");
    let mut encrypt = Command::new("sops")
        .args([
            "encrypt",
            "--input-type",
            "json",
            "--output-type",
            "json",
            "--age",
            &public,
            "/dev/stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    encrypt.stdin.take().unwrap().write_all(br#"{"provider":{"key":"fictional-fw-key"},"UNRELATED":"never-export-this", "empty":"", "multiline":"bad\nkey"}"#).unwrap();
    let encrypted = encrypt.wait_with_output().unwrap();
    assert!(encrypted.status.success());
    std::fs::write(&file, encrypted.stdout).unwrap();
    let link = dir.path().join("linked.json");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    let value = boblm_server::secrets::read_secret(
        link.to_str().unwrap(),
        &["provider".into(), "key".into()],
        dir.path(),
    )
    .await
    .unwrap();
    let mut child = tokio::process::Command::new("python3");
    child.arg("-c").arg("import os,sys; sys.exit(0 if os.environ['FIREWORKS_API_KEY']=='fictional-fw-key' and 'UNRELATED' not in os.environ else 1)").env("FIREWORKS_API_KEY",value);
    assert!(child.status().await.unwrap().success());
    for key in ["missing", "empty", "multiline"] {
        let error =
            boblm_server::secrets::read_secret(link.to_str().unwrap(), &[key.into()], dir.path())
                .await
                .err()
                .unwrap();
        assert!(error.contains(key) && error.contains("linked.json"));
        assert!(
            !error.contains("fictional-fw-key")
                && !error.contains("never-export-this")
                && !error.contains("bad\nkey")
        );
    }
    std::fs::remove_file(&identity).unwrap();
    let error = boblm_server::secrets::read_secret(
        link.to_str().unwrap(),
        &["provider".into(), "key".into()],
        dir.path(),
    )
    .await
    .err()
    .unwrap();
    assert!(
        error.contains("age identity")
            && error.contains("provider")
            && error.contains("linked.json")
    );
    assert!(!error.contains("fictional-fw-key"));
}
