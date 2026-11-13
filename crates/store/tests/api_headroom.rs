use boblm_store::Store;
use boblm_tuning::{ApiHeadroom, CODEX_ACCOUNT_KEY_API, CODEX_ACCOUNT_KEY_PLAN, MODEL_GPT_6_LUNA};
use serde_json::json;

#[test]
fn api_headroom_counts_uncached_tokens_and_only_projects_mid_turn_api_reviewers() {
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
        // Owner operations may create fixture states above any agent-spawn cap.
        store
            .create_session_in(boblm_store::NewSession {
                id,
                project_id: &project,
                name: id,
                agent: "codex",
                model: MODEL_GPT_6_LUNA,
                effort: Some("xhigh"),
                created_at,
            })
            .unwrap();
        store.set_claude_account_key(id, key).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE sessions SET role = ?1 WHERE id = ?2",
            rusqlite::params![role, id],
        )
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
    // Cache reads are huge here and in "fresh"; only input, cache writes and output count.
    call("busy", now - 50, 200, 5_000_000);
    call("recent", now - 1, 300, 400);
    call("fresh", now - 1, 0, 90_000_000);
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
            recent_tokens: 130 + 230 + 330 + 530 + 730 + 30,
            projected_tokens: 230,
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
            projected_tokens: 230,
            active_reviewers: 1,
        }
    );
}
