//! Credential requests against the real `sops` and `age` (both must be on PATH). The age key
//! used is a throwaway generated under target/ for these tests alone.
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use boblm_protocol::{Event, Priority};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::{app::App, secrets::SecretArgs};
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{Mutex, mpsc};
use tower::ServiceExt;

const NOW: i64 = 1_800_000_000_000;
const R1: &str = "30000000-0000-4000-8000-000000000001";
const R2: &str = "30000000-0000-4000-8000-000000000002";
const R3: &str = "30000000-0000-4000-8000-000000000003";
const VALUE: &str = "correct-horse-battery-STAPLE-9141";

struct Keys {
    public: String,
    stranger: String,
}
/// One throwaway age identity for this test binary; sops finds it through the environment.
fn keys() -> &'static Keys {
    static KEYS: OnceLock<Keys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/secret-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let generate = |name: &str| {
            let file = dir.join(name);
            let _ = std::fs::remove_file(&file);
            let out = Command::new("age-keygen")
                .arg("-o")
                .arg(&file)
                .output()
                .expect("age-keygen must be on PATH to run these tests");
            let text = String::from_utf8_lossy(&out.stderr).into_owned()
                + &String::from_utf8_lossy(&out.stdout);
            let public = text
                .split_whitespace()
                .find(|w| w.starts_with("age1"))
                .expect("age-keygen prints the public key")
                .to_owned();
            (file, public)
        };
        let (mine, public) = generate("mine.txt");
        let (_, stranger) = generate("stranger.txt");
        // SAFETY: set once, before any test starts a sops process.
        unsafe { std::env::set_var("SOPS_AGE_KEY_FILE", &mine) };
        Keys { public, stranger }
    })
}

struct Fake {
    events: mpsc::UnboundedReceiver<Event>,
    inputs: mpsc::UnboundedSender<String>,
}
#[async_trait]
impl Driver for Fake {
    async fn send(&mut self, _: &str, content: &str, _: Option<Priority>) -> io::Result<()> {
        self.inputs.send(content.into()).map_err(io::Error::other)
    }
    async fn next_event(&mut self) -> Option<Event> {
        self.events.recv().await
    }
    async fn configure(&mut self, _: &str, _: Option<&str>) -> io::Result<()> {
        Ok(())
    }
    async fn abort(&mut self) -> io::Result<()> {
        Ok(())
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Factory(Mutex<Option<Fake>>);
#[async_trait]
impl DriverFactory for Factory {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        Ok(Box::new(
            self.0
                .lock()
                .await
                .take()
                .ok_or_else(|| io::Error::other("second spawn"))?,
        ))
    }
}

struct Fixture {
    temp: tempfile::TempDir,
    store: Store,
    app: Arc<App>,
    _events: mpsc::UnboundedSender<Event>,
    inputs: mpsc::UnboundedReceiver<String>,
}
async fn fixture() -> Fixture {
    keys();
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let store = Store::open(temp.path().join("secrets.sqlite")).unwrap();
    store
        .create_session("s", work.to_str().unwrap(), 1)
        .unwrap();
    let (events_tx, events) = mpsc::unbounded_channel();
    let (inputs_tx, inputs) = mpsc::unbounded_channel();
    let factory = Arc::new(Factory(Mutex::new(Some(Fake {
        events,
        inputs: inputs_tx,
    }))));
    let app = App::open(store.clone(), factory, "secret-test-token-only".into())
        .await
        .unwrap();
    Fixture {
        temp,
        store,
        app,
        _events: events_tx,
        inputs,
    }
}
impl Fixture {
    fn work(&self) -> PathBuf {
        self.temp.path().join("work")
    }
    /// A SOPS file in the chat's folder with db.user in it.
    fn sops_file(&self, name: &str, recipient: &str, extra: &[&str]) -> PathBuf {
        let plain = self.temp.path().join(format!("plain-{name}"));
        std::fs::write(&plain, "db:\n  user: me\nother: plain-text-value\n").unwrap();
        let out = Command::new("sops")
            .args(["encrypt", "--age", recipient])
            .args(extra)
            .arg(&plain)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let file = self.work().join(name);
        std::fs::write(&file, out.stdout).unwrap();
        file
    }
    async fn next(&mut self) -> String {
        tokio::time::timeout(Duration::from_secs(3), self.inputs.recv())
            .await
            .expect("a message to the chat")
            .unwrap()
    }
    async fn nothing(&mut self) {
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            self.inputs.try_recv().is_err(),
            "the chat should hear nothing"
        );
    }
    async fn ask(&self, id: &str, file: &str, key: &[&str]) -> Result<serde_json::Value, String> {
        self.app
            .request_secret(
                "s",
                SecretArgs {
                    request_id: id.into(),
                    file: file.into(),
                    key: key.iter().map(|k| (*k).to_owned()).collect(),
                },
                NOW,
            )
            .await
    }
    /// Everything stored about the chat as one string: events and messages.
    fn everything_stored(&self) -> String {
        let reader = self.store.reader().unwrap();
        reader
            .events_since("s", 0, 1000)
            .unwrap()
            .iter()
            .map(|e| format!("{} {}", e.kind, e.payload))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
fn decrypt(file: &Path, key: &str) -> String {
    let out = Command::new("sops")
        .args(["decrypt", "--extract", key])
        .arg(file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test]
async fn a_saved_credential_lands_encrypted_and_neither_the_chat_nor_the_store_sees_it() {
    let mut f = fixture().await;
    let file = f.sops_file("secrets.yaml", &keys().public, &[]);
    let before_mode = std::fs::metadata(&file).unwrap().permissions();
    let asked = f
        .ask(R1, "secrets.yaml", &["db", "password"])
        .await
        .unwrap();
    assert_eq!(asked["state"], "pending");
    assert_eq!(asked["alreadyExisted"], false);
    assert_eq!(asked["key"], serde_json::json!(["db", "password"]));
    assert!(f.everything_stored().contains("secret_requested"));

    let reply = f
        .app
        .submit_secret(R1, Some(VALUE.into()), NOW + 1000)
        .await
        .unwrap();
    assert_eq!(reply.status, "saved", "{}", reply.message);

    assert_eq!(decrypt(&file, r#"["db"]["password"]"#), VALUE);
    assert_eq!(
        decrypt(&file, r#"["db"]["user"]"#),
        "me",
        "other keys are untouched"
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains(VALUE), "the value is encrypted in the file");
    assert_eq!(std::fs::metadata(&file).unwrap().permissions(), before_mode);
    let leftovers: Vec<_> = std::fs::read_dir(f.work())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".boblm-sops-"))
        .collect();
    assert!(leftovers.is_empty(), "the scratch folder is removed");

    let told = f.next().await;
    assert!(
        told.starts_with(&format!(
            "Credential request {R1}: the owner saved the value"
        )),
        "{told}"
    );
    assert!(!told.contains(VALUE));
    assert!(
        !f.everything_stored().contains(VALUE),
        "no event holds the value"
    );
    assert!(!reply.message.contains(VALUE));
}

#[tokio::test]
async fn requests_are_checked_before_the_owner_is_asked() {
    let f = fixture().await;
    f.sops_file("secrets.yaml", &keys().public, &[]);
    std::fs::write(f.work().join("plain.yaml"), "a: 1\n").unwrap();
    std::fs::write(f.work().join("notes.txt"), "hello").unwrap();
    for (file, key, expect) in [
        ("missing.yaml", vec!["a"], "not an existing file"),
        ("plain.yaml", vec!["a"], "not a SOPS-encrypted file"),
        ("notes.txt", vec!["a"], "not a .yaml, .yml or .json"),
        (
            "secrets.yaml",
            vec!["sops", "age"],
            "cannot start with \"sops\"",
        ),
        ("secrets.yaml", vec![], "1 to 16 object keys"),
        ("secrets.yaml", vec![""], "1 to 200 characters"),
    ] {
        let why = f.ask(R1, file, &key).await.unwrap_err();
        assert!(why.contains(expect), "{file} {key:?}: {why}");
    }
    let bad_id = f
        .app
        .request_secret(
            "s",
            SecretArgs {
                request_id: "nope".into(),
                file: "secrets.yaml".into(),
                key: vec!["a".into()],
            },
            NOW,
        )
        .await
        .unwrap_err();
    assert!(bad_id.contains("fresh UUID"), "{bad_id}");
    assert!(
        !f.everything_stored().contains("secret_requested"),
        "a refused request leaves nothing for the owner to see"
    );
    // Creation is idempotent and limited.
    f.ask(R1, "secrets.yaml", &["a"]).await.unwrap();
    assert_eq!(
        f.ask(R1, "secrets.yaml", &["a"]).await.unwrap()["alreadyExisted"],
        true
    );
    assert!(
        f.ask(R1, "secrets.yaml", &["b"]).await.is_err(),
        "same ID, different request"
    );
    for n in 0..4 {
        f.ask(
            &format!("30000000-0000-4000-8000-0000000001{n:02}"),
            "secrets.yaml",
            &["a"],
        )
        .await
        .unwrap();
    }
    let full = f.ask(R2, "secrets.yaml", &["a"]).await.unwrap_err();
    assert!(full.contains("waiting on 5 credential requests"), "{full}");
}

#[tokio::test]
async fn a_request_is_answered_once_and_declining_or_waiting_too_long_tells_the_chat() {
    let mut f = fixture().await;
    f.sops_file("secrets.yaml", &keys().public, &[]);
    f.ask(R1, "secrets.yaml", &["db", "password"])
        .await
        .unwrap();
    f.ask(R2, "secrets.yaml", &["db", "token"]).await.unwrap();

    let declined = f.app.submit_secret(R1, None, NOW + 1).await.unwrap();
    assert_eq!(declined.status, "canceled");
    let told = f.next().await;
    assert!(told.contains("declined"), "{told}");
    let again = f
        .app
        .submit_secret(R1, Some(VALUE.into()), NOW + 2)
        .await
        .unwrap();
    assert_eq!(again.status, "closed", "{}", again.message);
    f.nothing().await;

    // Nobody answers R2 in time: the sweep closes it and tells the chat once.
    let after = NOW + 11 * 60 * 1000;
    f.app.sweep_secrets(after).await;
    let told = f.next().await;
    assert!(told.contains("expired") && told.contains(R2), "{told}");
    f.app.sweep_secrets(after + 1000).await;
    f.nothing().await;
    let late = f
        .app
        .submit_secret(R2, Some(VALUE.into()), after + 2000)
        .await
        .unwrap();
    assert_eq!(late.status, "expired");
    assert!(late.message.contains("expired"), "{}", late.message);
    let unknown = f
        .app
        .submit_secret(R3, Some(VALUE.into()), after)
        .await
        .unwrap();
    assert_eq!(unknown.status, "closed");
}

#[tokio::test]
async fn a_save_that_cannot_work_changes_nothing_and_says_why() {
    let mut f = fixture().await;
    // Only keys named `secret` are encrypted, so `db.password` would be left in plain text.
    let partial = f.sops_file(
        "partial.yaml",
        &keys().public,
        &["--encrypted-regex", "^secret$"],
    );
    // Encrypted for someone else: this server cannot open it.
    let foreign = f.sops_file("foreign.yaml", &keys().stranger, &[]);
    f.ask(R1, "partial.yaml", &["db", "password"])
        .await
        .unwrap();
    f.ask(R2, "foreign.yaml", &["db", "password"])
        .await
        .unwrap();
    for (id, file, expect) in [(R1, &partial, "plain text"), (R2, &foreign, "SOPS refused")] {
        let before = std::fs::read(file).unwrap();
        let reply = f
            .app
            .submit_secret(id, Some(VALUE.into()), NOW + 1000)
            .await
            .unwrap();
        assert_eq!(reply.status, "failed");
        assert!(reply.message.contains(expect), "{}", reply.message);
        assert!(
            reply.message.contains("Nothing was changed"),
            "{}",
            reply.message
        );
        assert!(!reply.message.contains(VALUE));
        assert_eq!(
            std::fs::read(file).unwrap(),
            before,
            "the file is untouched"
        );
        let told = f.next().await;
        assert!(
            told.contains("could not be saved") && !told.contains(VALUE),
            "{told}"
        );
    }
    assert!(!f.everything_stored().contains(VALUE));
}

#[tokio::test]
async fn the_http_command_never_echoes_a_secret_back_in_an_error() {
    let f = fixture().await;
    f.sops_file("secrets.yaml", &keys().public, &[]);
    f.ask(R1, "secrets.yaml", &["db", "password"])
        .await
        .unwrap();
    let router = f.app.router(None);
    let post = |body: serde_json::Value| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/commands/submitSecret")
                        .header("Authorization", "Bearer secret-test-token-only")
                        .header("Content-Type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            (status, String::from_utf8_lossy(&bytes).into_owned())
        }
    };
    // Wrong shapes that carry the value in a wrong place or type.
    for body in [
        serde_json::json!({"requestId": R1, "value": VALUE, "cancel": VALUE}),
        serde_json::json!({"requestId": VALUE, "value": VALUE}),
        serde_json::json!({"requestId": R1, "value": VALUE, "extra": VALUE}),
        serde_json::json!({"requestId": R1, "value": VALUE, "cancel": true}),
        serde_json::json!({"requestId": R1}),
    ] {
        let (status, text) = post(body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {text}");
        assert!(!text.contains(VALUE), "{body}: {text}");
    }
    let (status, text) = post(serde_json::json!({"requestId": R1, "value": VALUE})).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let reply: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["status"], "saved");
    assert!(!text.contains(VALUE));
}
