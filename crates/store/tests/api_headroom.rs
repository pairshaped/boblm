use boblm_store::{PeerSpawn, Store};
use boblm_tuning::{ApiHeadroom, CODEX_ACCOUNT_KEY_API, CODEX_ACCOUNT_KEY_PLAN, MODEL_GPT_6_LUNA};
use serde_json::json;

#[test]
fn api_headroom_counts_cached_tokens_and_only_projects_mid_turn_api_reviewers() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("headroom.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("root", "/fixture", 1).unwrap();
    let project = store.reader().unwrap().sessions().unwrap()[0]
        .project_id
        .clone()
        .unwrap();
    let now = 1_000_000;
    let spawn = |id: &str, key: &str, role: &str, created_at: i64| {
        store
            .spawn_peer(PeerSpawn {
                request_id: id.into(),
                caller: "root".into(),
                arguments: json!({"requestId":id}).to_string(),
                project_id: project.clone(),
                agent: "codex".into(),
                model: MODEL_GPT_6_LUNA.into(),
                effort: "xhigh".into(),
                title: id.into(),
                content: "work".into(),
                created_at,
                parent_session_id: None,
                root_session_id: None,
                role: Some(role.into()),
                resolved: None,
                working_directory: None,
                account_key: Some(key.into()),
            })
            .unwrap();
    };
    spawn("busy", CODEX_ACCOUNT_KEY_API, "reviewer", 1);
    spawn("recent", CODEX_ACCOUNT_KEY_API, "reviewer", 1);
    spawn("old", CODEX_ACCOUNT_KEY_API, "reviewer", 1);
    spawn("archived", CODEX_ACCOUNT_KEY_API, "reviewer", 1);
    spawn("plan", CODEX_ACCOUNT_KEY_PLAN, "reviewer", now);
    spawn("other-role", CODEX_ACCOUNT_KEY_API, "implementer", now);
    spawn("fresh", CODEX_ACCOUNT_KEY_API, "reviewer", now);
    let call = |id: &str, at, tokens, cached| {
        store
            .record_event(
                id,
                "agent_event",
                json!({
                    "type":"token_usage", "model":MODEL_GPT_6_LUNA,
                    "input_tokens":tokens, "cached_input_tokens":cached,
                    "cache_write_input_tokens":10, "output_tokens":20
                }),
                at,
            )
            .unwrap();
    };
    call("busy", now - 100, 100, 200);
    call("busy", now - 50, 200, 300);
    call("recent", now - 1, 300, 400);
    call("old", now - 60_000, 400, 500);
    call("archived", now - 1, 500, 600);
    call("plan", now - 1, 600, 700);
    call("other-role", now - 1, 700, 800);
    store
        .publish_summary("busy", json!({"turnActive":true}))
        .unwrap();
    store
        .publish_summary("recent", json!({"turnActive":false}))
        .unwrap();
    store
        .publish_summary("archived", json!({"turnActive":true}))
        .unwrap();
    store.set_archived("archived", true).unwrap();
    assert_eq!(
        store.reader().unwrap().api_reviewer_headroom(now).unwrap(),
        ApiHeadroom {
            recent_tokens: 330 + 530 + 730 + 1_130 + 1_530,
            projected_tokens: 530,
            active_reviewers: 3,
        }
    );
    // A mid-turn chat reserves its last request even after that call ages out of the window.
    assert_eq!(
        store
            .reader()
            .unwrap()
            .api_reviewer_headroom(now + 60_001)
            .unwrap(),
        ApiHeadroom {
            recent_tokens: 0,
            projected_tokens: 530,
            active_reviewers: 1,
        }
    );
}
