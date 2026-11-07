use crate::app::App;
use boblm_protocol::client::ChatPresence;
use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Instant};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReloadArgs {
    /// Fresh UUID. An identical explicit retry must keep the same ID.
    request_id: String,
}
struct Receipt {
    caller: String,
    target: Option<String>,
    at: Instant,
    result: Value,
}
#[derive(Default)]
pub(crate) struct BrowserReloads {
    tabs: BTreeMap<String, (String, Instant)>,
    receipts: BTreeMap<String, Receipt>,
}
impl BrowserReloads {
    pub(crate) fn presence(&mut self, args: &ChatPresence, now: Instant) -> Option<String> {
        self.tabs
            .retain(|_, (_, at)| now.duration_since(*at) < boblm_tuning::NOTIFICATION_PRESENCE_TTL);
        let viewing = args
            .reload_capable
            .then_some(args.session_id.as_ref())
            .flatten();
        if let Some(session) = viewing {
            self.tabs
                .insert(args.client_id.clone(), (session.clone(), now));
        } else {
            self.tabs.remove(&args.client_id);
        }
        let mut reply = None;
        for (id, receipt) in &mut self.receipts {
            if receipt.target.as_deref() != Some(&args.client_id) {
                continue;
            }
            // Consume before sending the HTTP reply. Lost replies are never replayed.
            receipt.target = None;
            if viewing.is_some_and(|s| s == &receipt.caller)
                && now.duration_since(receipt.at) < boblm_tuning::BROWSER_RELOAD_TTL
                && reply.is_none()
            {
                reply = Some(id.clone());
            }
        }
        reply
    }
    fn request(&mut self, caller: &str, id: &str, now: Instant) -> Result<Value, String> {
        if let Some(receipt) = self.receipts.get(id) {
            if receipt.caller != caller {
                return Err(
                    "That reload request ID belongs to another chat. Use a fresh UUID.".into(),
                );
            }
            return Ok(receipt.result.clone());
        }
        if self.receipts.len() >= boblm_tuning::BROWSER_RELOAD_RECEIPT_LIMIT {
            return Err("The browser reload receipt limit was reached. Restart the server before requesting another reload.".into());
        }
        self.tabs
            .retain(|_, (_, at)| now.duration_since(*at) < boblm_tuning::NOTIFICATION_PRESENCE_TTL);
        let targets: Vec<_> = self
            .tabs
            .iter()
            .filter(|(_, (session, _))| session == caller)
            .map(|(id, _)| id.clone())
            .collect();
        let target = if targets.len() == 1 {
            targets.first().cloned()
        } else {
            None
        };
        let result = match &target {
            Some(_) => json!({"requestId":id,"accepted":true}),
            None => {
                json!({"requestId":id,"accepted":false,"reason":if targets.is_empty(){"no_focused_tab"}else{"ambiguous_tabs"}})
            }
        };
        self.receipts.insert(
            id.into(),
            Receipt {
                caller: caller.into(),
                target,
                at: now,
                result: result.clone(),
            },
        );
        Ok(result)
    }
}
impl App {
    pub async fn reload_ui(&self, caller: &str, args: ReloadArgs) -> Result<Value, String> {
        let id = boblm_runner::canonical_uuid(&args.request_id).map_err(|_| {
            "The reload request ID is not a UUID. Generate a fresh UUID and try again.".to_string()
        })?;
        self.browser_reloads
            .lock()
            .map_err(|_| {
                "The browser reload state is unavailable. Restart the server and try again."
                    .to_string()
            })?
            .request(caller, &id, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tab(client: &str, session: Option<&str>, capable: bool) -> ChatPresence {
        ChatPresence {
            client_id: client.into(),
            session_id: session.map(str::to_string),
            reload_capable: capable,
        }
    }
    #[test]
    fn reload_targets_only_the_unique_supported_tab_and_is_offered_once() {
        let mut state = BrowserReloads::default();
        let now = Instant::now();
        state.presence(&tab("a", Some("chat"), true), now);
        state.presence(&tab("b", Some("other"), true), now);
        state.presence(&tab("old", Some("chat"), false), now);
        assert_eq!(
            state.request("chat", "request", now).unwrap()["accepted"],
            true
        );
        assert_eq!(state.presence(&tab("b", Some("other"), true), now), None);
        assert_eq!(
            state.presence(&tab("a", Some("chat"), true), now),
            Some("request".into())
        );
        assert_eq!(
            state.request("chat", "request", now).unwrap()["accepted"],
            true
        );
        assert_eq!(state.presence(&tab("a", Some("chat"), true), now), None);
        assert!(state.request("other", "request", now).is_err());
    }
    #[test]
    fn reload_is_suppressed_without_a_target_or_with_two_focused_tabs() {
        let mut state = BrowserReloads::default();
        let now = Instant::now();
        assert_eq!(
            state.request("chat", "none", now).unwrap()["reason"],
            "no_focused_tab"
        );
        state.presence(&tab("a", Some("chat"), true), now);
        state.presence(&tab("b", Some("chat"), true), now);
        assert_eq!(
            state.request("chat", "two", now).unwrap()["reason"],
            "ambiguous_tabs"
        );
        state.presence(&tab("b", None, true), now);
        // A previously suppressed request cannot turn into a reload on retry.
        assert_eq!(
            state.request("chat", "two", now).unwrap()["accepted"],
            false
        );
        assert_eq!(state.presence(&tab("a", Some("chat"), true), now), None);
    }
    #[test]
    fn queued_reload_expires_and_never_follows_a_tab_to_another_chat_or_back() {
        let now = Instant::now();
        for next in [None, Some("other")] {
            let mut state = BrowserReloads::default();
            state.presence(&tab("a", Some("chat"), true), now);
            state.request("chat", "request", now).unwrap();
            assert_eq!(state.presence(&tab("a", next, true), now), None);
            assert_eq!(state.presence(&tab("a", Some("chat"), true), now), None);
        }
        let mut state = BrowserReloads::default();
        state.presence(&tab("a", Some("chat"), true), now);
        state.request("chat", "expired", now).unwrap();
        assert_eq!(
            state.presence(
                &tab("a", Some("chat"), true),
                now + boblm_tuning::BROWSER_RELOAD_TTL
            ),
            None
        );
        assert_eq!(
            state
                .request(
                    "chat",
                    "stale",
                    now + boblm_tuning::NOTIFICATION_PRESENCE_TTL * 2
                )
                .unwrap()["reason"],
            "no_focused_tab"
        );
    }
    struct NeverSpawn;
    #[async_trait::async_trait]
    impl boblm_runner::DriverFactory for NeverSpawn {
        async fn spawn(
            &self,
            _: &boblm_store::Session,
        ) -> std::io::Result<Box<dyn boblm_runner::Driver>> {
            panic!("a UI reload must not start an agent")
        }
    }
    #[tokio::test]
    async fn reload_tool_is_caller_bound_and_requires_an_agent_capability() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let temp = tempfile::tempdir().unwrap();
        let store = boblm_store::Store::open(temp.path().join("reload.sqlite")).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        let owner = "reload-test-owner-token";
        let app = App::open(store, std::sync::Arc::new(NeverSpawn), owner.into())
            .await
            .unwrap();
        app.browser_reloads
            .lock()
            .unwrap()
            .presence(&tab("a", Some("s"), true), Instant::now());
        let router = app.router(None);
        let request = |token: &str, args: Value| {
            Request::builder().method("POST").uri("/mcp/s").header("Host", "127.0.0.1")
                .header("Authorization", format!("Bearer {token}")).header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream").header("MCP-Protocol-Version", "2025-11-25")
                .body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"reload_ui","arguments":args}}).to_string())).unwrap()
        };
        let args = json!({"requestId":"10000000-0000-4000-8000-000000000001"});
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
        let result: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(result["result"]["isError"], false);
        let content: Value =
            serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(content["accepted"], true);
        let mut injection = args;
        injection["sessionId"] = json!("other");
        let response = router
            .oneshot(request(&capability, injection))
            .await
            .unwrap();
        let result: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(result["result"]["isError"], true);
    }
}
