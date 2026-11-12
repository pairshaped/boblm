//! Durable push delivery. An uncertain send is never automatically replayed.
use crate::app::{ApiError, ApiResult, App};
use axum::{Json, body::Bytes, extract::State};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use boblm_protocol::client::{
    ChatPresence, Empty, PushConfigReply, SubscribePush, UnsubscribePush,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Instant};
use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint;
use web_push_native::{
    Auth, WebPushBuilder, jwt_simple::algorithms::ES256KeyPair, p256::PublicKey,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PushConfig {
    pub private_key_file: PathBuf,
    /// A mailto: contact or public HTTPS URL for VAPID.
    pub subject: String,
    #[serde(default = "yes")]
    /// Deprecated compatibility setting. Accepted but ignored.
    pub agent_notices: bool,
    #[serde(default = "yes")]
    pub questions: bool,
    #[serde(default)]
    /// Deprecated compatibility setting. Accepted but ignored.
    pub completion: bool,
    #[serde(default = "yes")]
    /// Deprecated compatibility setting. Accepted but ignored.
    pub failures: bool,
}
fn yes() -> bool {
    true
}
impl PushConfig {
    fn allows(&self, category: &str) -> bool {
        category == "questions" && self.questions
    }
}
#[async_trait::async_trait]
trait PushSender: Send + Sync {
    fn public_key(&self) -> String;
    async fn send(
        &self,
        subscription: &boblm_store::Subscription,
        payload: Vec<u8>,
    ) -> Result<u16, String>;
}
struct WebPush {
    key: ES256KeyPair,
    public_key: String,
    subject: String,
    client: reqwest::Client,
}
#[async_trait::async_trait]
impl PushSender for WebPush {
    fn public_key(&self) -> String {
        self.public_key.clone()
    }
    async fn send(
        &self,
        subscription: &boblm_store::Subscription,
        payload: Vec<u8>,
    ) -> Result<u16, String> {
        let public = PublicKey::from_sec1_bytes(
            &URL_SAFE_NO_PAD
                .decode(&subscription.p256dh)
                .map_err(|e| format!("Could not prepare the push notification: {e}. Enable notifications again on that device."))?,
        )
        .map_err(|e| format!("Could not prepare the push notification: {e}. Enable notifications again on that device."))?;
        let auth = URL_SAFE_NO_PAD
            .decode(&subscription.auth)
            .map_err(|e| format!("Could not prepare the push notification: {e}. Enable notifications again on that device."))?;
        if auth.len() != 16 {
            return Err("The push subscription has an invalid authentication key. Enable notifications again on that device.".into());
        }
        let request = WebPushBuilder::new(
            subscription
                .endpoint
                .parse()
                .map_err(|e| format!("The stored push endpoint is invalid: {e}. Enable notifications again on that device."))?,
            public,
            Auth::clone_from_slice(&auth),
        )
        .with_valid_duration(boblm_tuning::NOTIFICATION_TTL)
        .with_vapid(&self.key, &self.subject)
        .build(payload)
        .map_err(|e| e.to_string())?;
        let (parts, body) = request.into_parts();
        let response=self.client.post(parts.uri.to_string()).headers(parts.headers).body(body).send().await.map_err(|e|format!("The push request did not finish: {}. Its outcome is uncertain; it will not be resent automatically.",e.without_url()))?;
        Ok(response.status().as_u16())
    }
}
#[derive(Default)]
pub(crate) struct Notifications {
    config: Option<PushConfig>,
    sender: Option<Arc<dyn PushSender>>,
    viewing: BTreeMap<String, (String, Instant)>,
}
impl Notifications {
    fn suppress(&mut self, sessions: &[&str], category: &str) -> Option<&'static str> {
        self.viewing
            .retain(|_, (_, at)| at.elapsed() < boblm_tuning::NOTIFICATION_PRESENCE_TTL);
        if !self.config.as_ref().is_some_and(|c| c.allows(category)) {
            Some("disabled_by_config")
        } else if self
            .viewing
            .values()
            .any(|(id, _)| sessions.contains(&id.as_str()))
        {
            Some("viewing_chat")
        } else {
            None
        }
    }
}
fn lock_error() -> ApiError {
    ApiError::internal(
        "The notification state was left locked by a failed operation. Restart the server.",
    )
}
fn subscription_id(endpoint: &str) -> String {
    format!("{:x}", Sha256::digest(endpoint.as_bytes()))
}
fn check_subscription(args: &SubscribePush) -> Result<(), ApiError> {
    let url = reqwest::Url::parse(&args.endpoint).map_err(|_| {
        ApiError::invalid("The push endpoint is not a valid URL. Enable notifications again.")
    })?;
    let host = url.host_str().unwrap_or("");
    let allowed = host == "fcm.googleapis.com"
        || host == "updates.push.services.mozilla.com"
        || host.ends_with(".push.apple.com")
        || host == "web.push.apple.com"
        || host.ends_with(".notify.windows.com");
    if url.scheme() != "https"
        || !allowed
        || url.port().is_some_and(|p| p != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::invalid(
            "This subscription does not point to a supported browser push service. Enable notifications again using Safari, Firefox, Chrome or Edge.",
        ));
    }
    let key = URL_SAFE_NO_PAD.decode(&args.p256dh).map_err(|_| {
        ApiError::invalid("The push encryption key is malformed. Enable notifications again.")
    })?;
    PublicKey::from_sec1_bytes(&key).map_err(|_| {
        ApiError::invalid(
            "The push encryption key is not a P-256 public key. Enable notifications again.",
        )
    })?;
    if URL_SAFE_NO_PAD
        .decode(&args.auth)
        .map_or(true, |a| a.len() != 16)
    {
        return Err(ApiError::invalid(
            "The push authentication key is malformed. Enable notifications again.",
        ));
    }
    Ok(())
}
impl App {
    pub(crate) fn start_notifications(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            loop {
                let Some(app) = weak.upgrade() else { break };
                app.deliver_notifications().await;
                drop(app);
                tokio::time::sleep(boblm_tuning::SCHEDULER_TICK).await;
            }
        });
        if let Ok(mut slot) = self.notification_task.lock()
            && let Some(old) = slot.replace(task)
        {
            old.abort();
        }
    }
    pub fn configure_notifications(&self, config: PushConfig) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let metadata=std::fs::metadata(&config.private_key_file).map_err(|e|format!("Could not read the notification key file: {e}. Check notifications.privateKeyFile in the config."))?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
            return Err("The notification key must be an owner-only regular file. Run chmod 600 on notifications.privateKeyFile.".into());
        }
        if !(config.subject.starts_with("mailto:") || config.subject.starts_with("https://")) {
            return Err("The notification subject must be a mailto: contact or public HTTPS URL. Update notifications.subject in the config.".into());
        }
        let pem = std::fs::read_to_string(&config.private_key_file).map_err(|e| {
            format!("Could not read the notification key: {e}. Check the configured key file.")
        })?;
        let key=ES256KeyPair::from_pem(&pem).map_err(|_|"The notification key is not a valid P-256 private key. Generate a PKCS#8 key with openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 and save it as the configured key file.".to_string())?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(boblm_tuning::NOTIFICATION_SEND_TIMEOUT)
            .build()
            .map_err(|e| {
                format!("Could not start the push HTTP client: {e}. Check the server's TLS setup.")
            })?;
        let public_key = PublicKey::from_sec1_bytes(&key.public_key().to_bytes()).map_err(|e| {
            format!("Could not derive the notification public key: {e}. Check the configured key.")
        })?;
        let public_key = URL_SAFE_NO_PAD.encode(public_key.to_encoded_point(false).as_bytes());
        let sender = Arc::new(WebPush {
            key,
            public_key,
            subject: config.subject.clone(),
            client,
        });
        let mut state = self
            .notifications
            .lock()
            .map_err(|_| lock_error().to_string())?;
        state.sender = Some(sender);
        state.config = Some(config);
        Ok(())
    }
    pub async fn deliver_notifications(&self) {
        if let Err(error) = self.drain_notifications().await {
            eprintln!("server: could not deliver notifications: {error}");
        }
    }
    async fn drain_notifications(&self) -> Result<(), ApiError> {
        let store = self.store.clone();
        let (pending, subscriptions) = tokio::task::spawn_blocking(move || {
            Ok::<_, boblm_store::Error>((
                store.reader()?.pending_notifications()?,
                store.reader()?.push_subscriptions()?,
            ))
        })
        .await
        .map_err(ApiError::internal)??;
        for id in pending {
            let store = self.store.clone();
            let lookup = id.clone();
            let Some(notice) =
                tokio::task::spawn_blocking(move || store.reader()?.notification(&lookup))
                    .await
                    .map_err(ApiError::internal)??
            else {
                continue;
            };
            let link = {
                let store = self.store.clone();
                let id = notice.session.clone();
                tokio::task::spawn_blocking(move || {
                    let reader = store.reader()?;
                    let root = reader
                        .sessions()?
                        .into_iter()
                        .find(|session| session.id == id)
                        .and_then(|session| session.root_session_id);
                    Ok::<_, boblm_store::Error>(root.unwrap_or(id))
                })
                .await
                .map_err(ApiError::internal)??
            };
            let (reason, sender) = {
                let mut state = self.notifications.lock().map_err(|_| lock_error())?;
                (
                    state.suppress(&[notice.session.as_str(), link.as_str()], &notice.category),
                    state.sender.clone(),
                )
            };
            let reason = reason.or(if subscriptions.is_empty() {
                Some("no_subscriptions")
            } else {
                None
            });
            if let Some(reason) = reason {
                let store = self.store.clone();
                tokio::task::spawn_blocking(move || {
                    store.settle_notification(&id, "suppressed", reason)
                })
                .await
                .map_err(ApiError::internal)??;
                continue;
            }
            let Some(sender) = sender else { continue };
            let payload=serde_json::to_vec(&serde_json::json!({"id":notice.id,"sessionId":link,"questionId":notice.question_id,"title":notice.title,"body":notice.message})).map_err(ApiError::internal)?;
            let mut state = "sent";
            for subscription in &subscriptions {
                let (store, n, s) = (self.store.clone(), id.clone(), subscription.id.clone());
                if !tokio::task::spawn_blocking(move || store.claim_push_delivery(&n, &s))
                    .await
                    .map_err(ApiError::internal)??
                {
                    state = "uncertain";
                    continue;
                }
                let outcome = if payload.len() <= boblm_tuning::MAX_NOTIFICATION_PAYLOAD_BYTES {
                    sender.send(subscription, payload.clone()).await
                } else {
                    Err(
                        "The notification exceeded the push payload limit. Shorten the message."
                            .into(),
                    )
                };
                let delivered = match outcome {
                    Ok(200..=299) => "sent",
                    Ok(404 | 410) => "expired",
                    Ok(_) => "failed",
                    Err(error) => {
                        eprintln!("server: push delivery: {error}");
                        "uncertain"
                    }
                };
                if delivered != "sent" {
                    state = "uncertain";
                }
                let (store, n, s) = (self.store.clone(), id.clone(), subscription.id.clone());
                tokio::task::spawn_blocking(move || {
                    store.finish_push_delivery(&n, &s, delivered)?;
                    if delivered == "expired" {
                        store.unsubscribe_push(&s)?;
                    }
                    Ok::<_, boblm_store::Error>(())
                })
                .await
                .map_err(ApiError::internal)??;
            }
            let store = self.store.clone();
            tokio::task::spawn_blocking(move || store.settle_notification(&id, state, ""))
                .await
                .map_err(ApiError::internal)??;
        }
        Ok(())
    }
}
pub async fn config(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<PushConfigReply> {
    let _: Empty = crate::app::body(&bytes)?;
    let public_key = app
        .notifications
        .lock()
        .map_err(|_| lock_error())?
        .sender
        .as_ref()
        .map(|s| s.public_key());
    Ok(Json(PushConfigReply { public_key }))
}
pub async fn subscribe(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<Empty> {
    let args: SubscribePush = crate::app::body(&bytes)?;
    check_subscription(&args)?;
    if app
        .notifications
        .lock()
        .map_err(|_| lock_error())?
        .sender
        .is_none()
    {
        return Err(ApiError::invalid(
            "Push notifications are not configured on this server. Add notifications to the server config first.",
        ));
    }
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || {
        store.subscribe_push(boblm_store::PushSubscription {
            id: &subscription_id(&args.endpoint),
            endpoint: &args.endpoint,
            p256dh: &args.p256dh,
            auth: &args.auth,
            created_at: crate::schedule::now_ms(),
        })
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(Empty {}))
}
pub async fn unsubscribe(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<Empty> {
    let args: UnsubscribePush = crate::app::body(&bytes)?;
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || store.unsubscribe_push(&subscription_id(&args.endpoint)))
        .await
        .map_err(ApiError::internal)??;
    Ok(Json(Empty {}))
}
pub async fn presence(
    State(app): State<Arc<App>>,
    bytes: Bytes,
) -> ApiResult<boblm_protocol::client::ChatPresenceReply> {
    let args: ChatPresence = crate::app::body(&bytes)?;
    let mut state = app.notifications.lock().map_err(|_| lock_error())?;
    if let Some(session) = &args.session_id {
        state
            .viewing
            .insert(args.client_id.clone(), (session.clone(), Instant::now()));
    } else {
        state.viewing.remove(&args.client_id);
    }
    drop(state);
    let mut reloads = app.browser_reloads.lock().map_err(|_| {
        ApiError::internal(
            "The browser reload state is unavailable. Restart the server and try again.",
        )
    })?;
    Ok(Json(boblm_protocol::client::ChatPresenceReply {
        reload_id: reloads.presence(&args, Instant::now()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use boblm_runner::{Driver, DriverFactory};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct NeverSpawn;
    #[async_trait::async_trait]
    impl DriverFactory for NeverSpawn {
        async fn spawn(&self, _: &boblm_store::Session) -> std::io::Result<Box<dyn Driver>> {
            panic!("notification tools must not start an agent")
        }
    }
    struct FakeSender {
        calls: AtomicUsize,
        status: Option<u16>,
        payloads: std::sync::Mutex<Vec<Vec<u8>>>,
    }
    #[async_trait::async_trait]
    impl PushSender for FakeSender {
        fn public_key(&self) -> String {
            "test-public-key".into()
        }
        async fn send(
            &self,
            _: &boblm_store::Subscription,
            payload: Vec<u8>,
        ) -> Result<u16, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.payloads.lock().unwrap().push(payload);
            self.status
                .ok_or("The fake connection ended after sending.".into())
        }
    }
    fn preferences() -> PushConfig {
        PushConfig {
            private_key_file: "/unused".into(),
            subject: "mailto:test@example.com".into(),
            agent_notices: true,
            questions: true,
            completion: false,
            failures: true,
        }
    }
    fn ask(store: &boblm_store::Store, id: u8) -> String {
        let question = format!("10000000-0000-4000-8000-{id:012}");
        store.ask(&question, "s", "Which way?", &[], 1).unwrap();
        format!("question:{question}")
    }
    #[test]
    fn legacy_notification_config_parses_but_only_questions_are_allowed() {
        let config: PushConfig = serde_json::from_value(serde_json::json!({
            "privateKeyFile": "/unused", "subject": "mailto:test@example.com",
            "agentNotices": true, "questions": false, "completion": true, "failures": true
        }))
        .unwrap();
        for category in ["agent", "completion", "failures", "questions"] {
            assert!(!config.allows(category));
        }
    }
    async fn fixture() -> (
        tempfile::TempDir,
        boblm_store::Store,
        Arc<App>,
        Arc<FakeSender>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let store = boblm_store::Store::open(temp.path().join("notices.sqlite")).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        let app = App::open(
            store.clone(),
            Arc::new(NeverSpawn),
            "notification-test-owner-token".into(),
        )
        .await
        .unwrap();
        let fake = Arc::new(FakeSender {
            calls: AtomicUsize::new(0),
            status: Some(201),
            payloads: std::sync::Mutex::new(Vec::new()),
        });
        {
            let mut state = app.notifications.lock().unwrap();
            state.config = Some(preferences());
            state.sender = Some(fake.clone());
        }
        store
            .subscribe_push(boblm_store::PushSubscription {
                id: "device",
                endpoint: "https://web.push.apple.com/test",
                p256dh: "fake",
                auth: "fake",
                created_at: 1,
            })
            .unwrap();
        (temp, store, app, fake)
    }
    #[tokio::test]
    async fn notification_retries_are_durable_deduplicated_and_never_start_an_agent() {
        let (_temp, _store, app, fake) = fixture().await;
        let id = ask(&_store, 1);
        assert_eq!(ask(&_store, 1), id);
        assert!(
            _store
                .ask("10000000-0000-4000-8000-000000000001", "s", "Other", &[], 1)
                .is_err()
        );
        app.drain_notifications().await.unwrap();
        app.drain_notifications().await.unwrap();
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            _store
                .reader()
                .unwrap()
                .notification(&id)
                .unwrap()
                .unwrap()
                .state,
            "sent"
        );
        app.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn preferences_and_current_viewing_suppress_even_an_accepted_notice() {
        let (_temp, _store, app, fake) = fixture().await;
        let id = ask(&_store, 2);
        app.notifications
            .lock()
            .unwrap()
            .viewing
            .insert("browser".into(), ("s".into(), Instant::now()));
        app.drain_notifications().await.unwrap();
        assert_eq!(
            _store
                .reader()
                .unwrap()
                .notification(&id)
                .unwrap()
                .unwrap()
                .reason,
            "viewing_chat"
        );
        app.notifications.lock().unwrap().viewing.clear();
        let id = ask(&_store, 3);
        app.notifications
            .lock()
            .unwrap()
            .config
            .as_mut()
            .unwrap()
            .questions = false;
        app.drain_notifications().await.unwrap();
        assert_eq!(
            _store
                .reader()
                .unwrap()
                .notification(&id)
                .unwrap()
                .unwrap()
                .reason,
            "disabled_by_config"
        );
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
        app.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn a_crash_after_a_claim_never_replays_that_push_and_expired_devices_are_removed() {
        let (_temp, _store, app, fake) = fixture().await;
        let id = ask(&_store, 4);
        assert!(app.store.claim_push_delivery(&id, "device").unwrap());
        app.drain_notifications().await.unwrap();
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            app.store
                .reader()
                .unwrap()
                .notification(&id)
                .unwrap()
                .unwrap()
                .state,
            "uncertain"
        );
        let expired = Arc::new(FakeSender {
            calls: AtomicUsize::new(0),
            status: Some(410),
            payloads: std::sync::Mutex::new(Vec::new()),
        });
        app.notifications.lock().unwrap().sender = Some(expired.clone());
        ask(&_store, 5);
        app.drain_notifications().await.unwrap();
        assert_eq!(expired.calls.load(Ordering::SeqCst), 1);
        assert!(
            app.store
                .reader()
                .unwrap()
                .push_subscriptions()
                .unwrap()
                .is_empty()
        );
        app.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn a_subagent_question_notification_links_to_its_root() {
        let (_temp, store, app, fake) = fixture().await;
        store.create_session("root", "/tmp", 2).unwrap();
        let project = store
            .reader()
            .unwrap()
            .sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.id == "root")
            .and_then(|session| session.project_id)
            .unwrap();
        let child = "10000000-0000-4000-8000-000000000811";
        store
            .spawn_peer(boblm_store::PeerSpawn {
                request_id: child.into(),
                caller: "root".into(),
                arguments: format!(r#"{{"title":"Worker","requestId":"{child}"}}"#),
                project_id: project,
                agent: "claude".into(),
                model: "sonnet".into(),
                effort: "high".into(),
                title: "Worker".into(),
                content: "work".into(),
                created_at: 3,
                parent_session_id: Some("root".into()),
                root_session_id: Some("root".into()),
                role: Some("implementer".into()),
                resolved: None,
                working_directory: None,
                account_key: None,
            })
            .unwrap();
        let question = "20000000-0000-4000-8000-000000000811";
        store.ask(question, child, "Which way?", &[], 4).unwrap();
        app.drain_notifications().await.unwrap();
        let payload: serde_json::Value = {
            let payloads = fake.payloads.lock().unwrap();
            serde_json::from_slice(payloads.last().expect("the question notification was sent"))
                .unwrap()
        };
        assert_eq!(payload["sessionId"], "root");
        assert_eq!(payload["questionId"], question);
        app.shutdown().await.unwrap();
    }

    #[test]
    fn encrypted_payloads_decrypt_for_the_subscribing_browser_and_vapid_uses_the_same_key() {
        use web_push_native::p256::{SecretKey, elliptic_curve::rand_core::OsRng};
        let secret = SecretKey::random(&mut OsRng);
        let auth = Auth::clone_from_slice(&[7; 16]);
        let key = ES256KeyPair::generate();
        let request = WebPushBuilder::new(
            "https://web.push.apple.com/test".parse().unwrap(),
            secret.public_key(),
            auth,
        )
        .with_vapid(&key, "mailto:test@example.com")
        .build(b"hello".to_vec())
        .unwrap();
        assert!(
            request.headers()["authorization"]
                .to_str()
                .unwrap()
                .starts_with("vapid ")
        );
        assert_eq!(
            web_push_native::decrypt(request.into_body(), &secret, &auth).unwrap(),
            b"hello"
        );
    }
}

#[cfg(test)]
mod authentication_tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;
    struct NeverSpawn;
    #[async_trait::async_trait]
    impl boblm_runner::DriverFactory for NeverSpawn {
        async fn spawn(
            &self,
            _: &boblm_store::Session,
        ) -> std::io::Result<Box<dyn boblm_runner::Driver>> {
            panic!("notification authentication must not start agents")
        }
    }
    #[tokio::test]
    async fn removed_notification_tool_rejects_calls_without_creating_notifications() {
        let temp = tempfile::tempdir().unwrap();
        let store = boblm_store::Store::open(temp.path().join("auth.sqlite")).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        let owner = "notification-auth-test-token";
        let app = App::open(store.clone(), Arc::new(NeverSpawn), owner.into())
            .await
            .unwrap();
        let router = app.router(None);
        let request = |token: &str, args: serde_json::Value| {
            Request::builder().method("POST").uri("/mcp/s").header("Host","127.0.0.1").header("Authorization",format!("Bearer {token}")).header("Content-Type","application/json").header("Accept","application/json, text/event-stream").header("MCP-Protocol-Version","2025-11-25").body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"notify","arguments":args}}).to_string())).unwrap()
        };
        let args = json!({"notificationId":"20000000-0000-4000-8000-000000000001","title":"Done","message":"Ready to review."});
        assert_eq!(
            router
                .clone()
                .oneshot(request(owner, args.clone()))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let capability = crate::mcp::session_token(owner, "s");
        let response = router
            .clone()
            .oneshot(request(&capability, args.clone()))
            .await
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(value.get("error").is_some() || value["result"]["isError"] == true);
        assert!(
            store
                .reader()
                .unwrap()
                .notification("20000000-0000-4000-8000-000000000001")
                .unwrap()
                .is_none()
        );
        let mut injected = args;
        injected["sessionId"] = "other-chat".into();
        let response = router
            .oneshot(request(&capability, injected))
            .await
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(value.get("error").is_some() || value["result"]["isError"] == true);
        app.shutdown().await.unwrap();
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    #[test]
    fn vapid_pkcs8_keys_produce_the_uncompressed_browser_public_key() {
        let key = ES256KeyPair::generate();
        let pem = key.to_pem().unwrap();
        let loaded = ES256KeyPair::from_pem(&pem).unwrap();
        let public = PublicKey::from_sec1_bytes(&loaded.public_key().to_bytes()).unwrap();
        let encoded = URL_SAFE_NO_PAD.encode(public.to_encoded_point(false).as_bytes());
        assert_eq!(URL_SAFE_NO_PAD.decode(encoded).unwrap().len(), 65);
        assert_eq!(key.public_key().to_bytes(), loaded.public_key().to_bytes());
    }
}
