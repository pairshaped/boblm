use boblm_pi_driver::{PiOptions, PiSession};
use boblm_protocol::{Event, LifecycleState};
use std::{path::Path, time::Duration};
use tokio::{process::Command, time::timeout};
const MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash-1017";
fn fake(mode: &str) -> Command {
    let mut c = Command::new("python3");
    c.arg("-u")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_pi.py"))
        .env("FAKE_MODE", mode);
    c
}
async fn start(mode: &str) -> (tempfile::TempDir, PiSession) {
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let s = PiSession::spawn_command(
        fake(mode),
        PiOptions {
            cwd: Path::new(env!("CARGO_MANIFEST_DIR")),
            state_dir: dir.path(),
            resume: None,
            model: MODEL,
            effort: Some("xhigh"),
            api_key: "fictional-provider-key",
            mcp: None,
        },
    )
    .await
    .unwrap();
    (dir, s)
}
async fn next(s: &mut PiSession) -> Event {
    timeout(Duration::from_secs(5), s.events.recv())
        .await
        .unwrap()
        .expect("Pi event")
}
async fn result(s: &mut PiSession) -> Vec<Event> {
    let mut all = Vec::new();
    loop {
        let e = next(s).await;
        let done = matches!(e, Event::Result { .. });
        all.push(e);
        if done {
            return all;
        }
    }
}
fn started(events: &[Event], id: &str) -> bool {
    events.iter().any(|e| matches!(e, Event::CommandLifecycle {command_uuid, state: LifecycleState::Started} if command_uuid==id))
}
#[tokio::test]
async fn pi_consumption_requires_the_user_message_not_a_successful_response() {
    let (_dir, mut s) = start("normal").await;
    assert!(matches!(
        next(&mut s).await,
        Event::System {
            session_id: Some(_),
            ..
        }
    ));
    s.send_message("one", "hello", None).await.unwrap();
    let events = result(&mut s).await;
    assert!(started(&events, "one"));
    assert!(events.iter().any(
        |e| matches!(e, Event::Assistant {message} if message["content"][0]["text"]=="echo: hello")
    ));
    s.stop().await.unwrap();
    let (_dir, mut s) = start("ack_only").await;
    next(&mut s).await;
    s.send_message("two", "hello", None).await.unwrap();
    let mut events = Vec::new();
    while let Some(e) = timeout(Duration::from_secs(5), s.events.recv())
        .await
        .unwrap()
    {
        events.push(e);
    }
    assert!(
        !started(&events, "two"),
        "an RPC success must not acknowledge delivery"
    );
    s.stop().await.unwrap();
}

#[tokio::test]
async fn pi_steering_and_late_steering_consume_the_right_duplicate_text() {
    let (_dir, mut s) = start("hold").await;
    next(&mut s).await;
    s.send_message("first", "same", None).await.unwrap();
    assert!(
        matches!(next(&mut s).await, Event::CommandLifecycle {command_uuid,state:LifecycleState::Started} if command_uuid=="first")
    );
    s.send_message("second", "same", Some(boblm_protocol::Priority::Now))
        .await
        .unwrap();
    let events = result(&mut s).await;
    assert!(started(&events, "second"));
    assert!(!started(&events, "first"));
    s.send_message("late", "after", Some(boblm_protocol::Priority::Now))
        .await
        .unwrap();
    assert!(started(&result(&mut s).await, "late"));
    s.stop().await.unwrap();
}
#[tokio::test]
async fn pi_unknown_or_unmatched_events_do_not_consume_input() {
    let (_dir, mut s) = start("unmatched").await;
    next(&mut s).await;
    s.send_message("held", "wanted", None).await.unwrap();
    let events = result(&mut s).await;
    assert!(!started(&events, "held"));
    s.stop().await.unwrap();
}
#[tokio::test]
async fn pi_refusal_and_provider_failure_are_distinct_from_consumption() {
    let (_dir, mut s) = start("normal").await;
    next(&mut s).await;
    s.send_message("r", "refuse", None).await.unwrap();
    let reason = next(&mut s).await;
    assert!(
        matches!(reason,Event::Assistant {message} if message["content"][0]["text"].as_str().unwrap().contains("no model"))
    );
    assert!(matches!(
        next(&mut s).await,
        Event::CommandLifecycle {
            state: LifecycleState::Refused,
            ..
        }
    ));
    s.send_message("f", "fail", None).await.unwrap();
    let events = result(&mut s).await;
    assert!(started(&events, "f"));
    assert!(matches!(
        events.last(),
        Some(Event::Result {
            is_error: Some(true),
            ..
        })
    ));
    assert!(
        !serde_json::to_string(&events)
            .unwrap()
            .contains("fictional-provider-key")
    );
    s.stop().await.unwrap();
}
#[tokio::test]
async fn pi_abort_waits_for_the_agent_end_and_keeps_unconsumed_steers_uncertain() {
    let (_dir, mut s) = start("hold_forever").await;
    next(&mut s).await;
    s.send_message("first", "slow", None).await.unwrap();
    next(&mut s).await;
    s.send_message("steer", "later", Some(boblm_protocol::Priority::Now))
        .await
        .unwrap();
    s.request_interrupt().await.unwrap();
    let events = result(&mut s).await;
    assert!(!started(&events, "steer"));
    assert!(matches!(events.last(),Some(Event::Result {subtype:Some(t),..}) if t=="interrupted"));
    assert!(
        timeout(Duration::from_secs(5), s.events.recv())
            .await
            .unwrap()
            .is_none(),
        "abort must discard Pi's in-memory queue by stopping the child"
    );
    s.stop().await.unwrap();
}
#[tokio::test]
async fn pi_compact_clear_and_usage_follow_real_events() {
    let (_dir, mut s) = start("normal").await;
    next(&mut s).await;
    let first = s.session_id();
    s.send_message("a", "hello", None).await.unwrap();
    let events = result(&mut s).await;
    let usage_events = events
        .iter()
        .filter(|event| {
            matches!(event, Event::Assistant { message }
                if message["usage"]["input_tokens"] == 42
                    && message["usage"]["cache_read_input_tokens"] == 10
                    && message["pi_usage"]["totalTokens"] == 55)
        })
        .count();
    assert_eq!(
        usage_events, 1,
        "Pi must emit one assistant usage event per message"
    );
    s.send_message("c", "/compact", None).await.unwrap();
    let events = result(&mut s).await;
    assert!(started(&events, "c"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::System { subtype:Some(s), .. } if s == "compact_started"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Event::System {subtype:Some(t),..} if t=="compact_boundary"))
    );
    s.send_message("clear", "/clear", None).await.unwrap();
    let events = result(&mut s).await;
    assert_ne!(first, s.session_id());
    assert!(events.iter().any(|e|matches!(e,Event::CommandLifecycle {command_uuid,state:LifecycleState::Completed} if command_uuid=="clear")));
    s.stop().await.unwrap();
    let (_dir, mut s) = start("compact_ack_only").await;
    next(&mut s).await;
    s.send_message("c", "/compact", None).await.unwrap();
    while let Some(e) = timeout(Duration::from_secs(5), s.events.recv())
        .await
        .unwrap()
    {
        assert!(!matches!(e, Event::CommandLifecycle { .. }));
    }
    s.stop().await.unwrap();
}
#[tokio::test]
async fn pi_resume_checks_the_file_and_reported_identity() {
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let path = dir.path().join("old.jsonl");
    std::fs::write(&path, "fake session").unwrap();
    for (mode, good) in [("normal", true), ("wrong_resume", false)] {
        let options = PiOptions {
            cwd: dir.path(),
            state_dir: dir.path(),
            resume: Some(path.to_str().unwrap()),
            model: MODEL,
            effort: None,
            api_key: "fictional-provider-key",
            mcp: None,
        };
        let session = PiSession::spawn_command(fake(mode), options).await;
        if good {
            let s = session.unwrap();
            assert_eq!(s.session_id().as_deref(), path.to_str());
            s.stop().await.unwrap();
        } else {
            assert!(
                session
                    .err()
                    .unwrap()
                    .to_string()
                    .to_lowercase()
                    .contains("no new conversation was started")
            );
        }
    }
    std::fs::remove_file(&path).unwrap();
    let options = PiOptions {
        cwd: dir.path(),
        state_dir: dir.path(),
        resume: Some(path.to_str().unwrap()),
        model: MODEL,
        effort: None,
        api_key: "fictional-provider-key",
        mcp: None,
    };
    assert!(
        PiSession::spawn_command(fake("normal"), options)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("no new conversation was started")
    );
}
#[tokio::test]
async fn pi_tool_results_streams_and_questions_use_the_shared_contract() {
    let (_dir, mut s) = start("normal").await;
    next(&mut s).await;
    s.send_message("t", "tool", None).await.unwrap();
    let events = result(&mut s).await;
    assert!(events.iter().any(|e|matches!(e,Event::Assistant {message} if message["content"][0]["type"]=="tool_use" && message["content"][0]["id"]=="call-1")));
    assert!(events.iter().any(|e|matches!(e,Event::User {message} if message["content"][0]["tool_use_id"]=="call-1" && message["content"][0]["is_error"]==true)));
    assert!(events.iter().any(|e| matches!(e,Event::Assistant {message} if message["content"][0]["text"]=="tool finished")), "an iteration must not finish the owner turn");
    s.send_message("q", "ask", None).await.unwrap();
    let events = result(&mut s).await;
    assert!(events.iter().any(
        |e| matches!(e,Event::Assistant {message} if message["content"][0]["text"]=="canceled")
    ));
    s.stop().await.unwrap();
}

#[test]
fn pi_peer_tools_extension_uses_http_and_does_not_wait_for_the_owner() {
    let output = std::process::Command::new("node")
        .arg("--test")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/peer-tools.test.mjs"
        ))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[tokio::test]
async fn pi_startup_failures_and_drop_stop_the_process_group() {
    use boblm_pi_driver::McpEndpoint;
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    for mode in ["exit_start", "bad_effort", "missing_tools"] {
        let options = PiOptions {
            cwd: dir.path(),
            state_dir: dir.path(),
            resume: None,
            model: MODEL,
            effort: Some("xhigh"),
            api_key: "fictional-provider-key",
            mcp: Some(McpEndpoint {
                url: "http://127.0.0.1:1/mcp/test",
                token: "fictional-capability",
            }),
        };
        let error = PiSession::spawn_command(fake(mode), options)
            .await
            .err()
            .expect("startup rejected");
        assert!(error.to_string().contains("Pi"));
    }
    let pid = dir.path().join("pids");
    let mut command = fake("fork_child");
    command.env("FAKE_PIDS", &pid);
    let s = PiSession::spawn_command(
        command,
        PiOptions {
            cwd: dir.path(),
            state_dir: dir.path(),
            resume: None,
            model: MODEL,
            effort: Some("xhigh"),
            api_key: "fictional-provider-key",
            mcp: None,
        },
    )
    .await
    .unwrap();
    let pids = std::fs::read_to_string(pid).unwrap();
    drop(s);
    for pid in pids.lines() {
        let path = format!("/proc/{pid}/stat");
        for _ in 0..100 {
            let state = std::fs::read_to_string(&path).ok();
            if state
                .as_ref()
                .is_none_or(|s| s.split_whitespace().nth(2) == Some("Z"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            std::fs::read_to_string(&path)
                .ok()
                .as_ref()
                .is_none_or(|s| s.split_whitespace().nth(2) == Some("Z")),
            "process {pid} survived drop"
        );
    }
}

#[tokio::test]
async fn pi_control_replies_cannot_fabricate_abort_or_clear_completion() {
    let (_dir, mut s) = start("abort_ack_only").await;
    next(&mut s).await;
    s.send_message("a", "slow", None).await.unwrap();
    next(&mut s).await;
    s.request_interrupt().await.unwrap();
    while let Some(e) = timeout(Duration::from_secs(5), s.events.recv())
        .await
        .unwrap()
    {
        assert!(!matches!(e, Event::Result { .. }));
    }
    s.stop().await.unwrap();
    let (_dir, mut s) = start("clear_same_file").await;
    next(&mut s).await;
    assert!(
        s.send_message("c", "/clear", None)
            .await
            .unwrap_err()
            .to_string()
            .contains("did not create a different conversation")
    );
    assert!(s.events.try_recv().is_err());
    s.stop().await.unwrap();
}

#[tokio::test]
async fn pi_project_settings_cannot_enable_hidden_retry_or_compaction() {
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    std::fs::create_dir(dir.path().join(".pi")).unwrap();
    for settings in [
        r#"{"retry":{"enabled":true}}"#,
        r#"{"retry":{"provider":{"maxRetries":0}}}"#,
        r#"{"retry":{"provider":{"maxRetries":3}}}"#,
        r#"{"compaction":{"enabled":true}}"#,
    ] {
        std::fs::write(dir.path().join(".pi/settings.json"), settings).unwrap();
        let error = PiSession::spawn_command(
            fake("normal"),
            PiOptions {
                cwd: dir.path(),
                state_dir: &dir.path().join("state"),
                resume: None,
                model: MODEL,
                effort: None,
                api_key: "fictional-provider-key",
                mcp: None,
            },
        )
        .await
        .err()
        .expect("project policy rejected");
        assert!(error.to_string().contains(".pi/settings.json"));
    }
    std::fs::write(
        dir.path().join(".pi/settings.json"),
        r#"{"retry":{"provider":{"maxRetries":2}}}"#,
    )
    .unwrap();
    let session = PiSession::spawn_command(
        fake("normal"),
        PiOptions {
            cwd: dir.path(),
            state_dir: &dir.path().join("state"),
            resume: None,
            model: MODEL,
            effort: None,
            api_key: "fictional-provider-key",
            mcp: None,
        },
    )
    .await
    .unwrap();
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("state/settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings["retry"]["enabled"], false);
    assert_eq!(
        settings["retry"]["provider"]["maxRetries"],
        boblm_tuning::PI_PROVIDER_MAX_RETRIES
    );
    assert_eq!(settings["compaction"]["enabled"], false);
    session.stop().await.unwrap();
}

#[tokio::test]
async fn pi_context_overflow_is_a_distinct_error_result() {
    let (_dir, mut s) = start("overflow").await;
    next(&mut s).await;
    s.send_message("overflow", "fail", None).await.unwrap();
    let events = result(&mut s).await;
    assert!(
        matches!(
            events.last(),
            Some(Event::Result {
                subtype: Some(subtype),
                is_error: Some(true),
            }) if subtype == "context_overflow"
        ),
        "{events:?}"
    );
    s.stop().await.unwrap();
}

#[tokio::test]
async fn pi_provider_errors_report_the_cause_without_a_json_or_stack_dump() {
    let (_dir, mut s) = start("json_error").await;
    next(&mut s).await;
    s.send_message("e", "fail", None).await.unwrap();
    let events = result(&mut s).await;
    let text = events
        .iter()
        .filter_map(|e| match e {
            Event::Assistant { message } => message["content"][0]["text"].as_str(),
            _ => None,
        })
        .collect::<String>();
    assert!(text.contains("429: rate limit [redacted]"));
    assert!(!text.contains("provider.js") && !text.contains('{'));
    s.stop().await.unwrap();
}

#[tokio::test]
async fn pi_simultaneous_duplicate_messages_are_consumed_in_fifo_order() {
    for mode in ["duplicates", "duplicate_refusal"] {
        let (_dir, mut s) = start(mode).await;
        next(&mut s).await;
        s.send_message("first", "same", None).await.unwrap();
        assert!(
            s.events.try_recv().is_err(),
            "the fake must hold the first acknowledgment"
        );
        s.send_message("second", "same", Some(boblm_protocol::Priority::Now))
            .await
            .unwrap();
        if mode == "duplicate_refusal" {
            assert!(matches!(next(&mut s).await, Event::Assistant { .. }));
            assert!(
                matches!(next(&mut s).await, Event::CommandLifecycle {command_uuid,state:LifecycleState::Refused} if command_uuid=="second")
            );
            s.send_message("third", "release", None).await.unwrap();
        }
        let events = result(&mut s).await;
        let consumed: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                Event::CommandLifecycle {
                    command_uuid,
                    state: LifecycleState::Started,
                } => Some(command_uuid.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            consumed,
            if mode == "duplicates" {
                vec!["first", "second"]
            } else {
                vec!["first", "third"]
            }
        );
        s.stop().await.unwrap();
    }
}

#[tokio::test]
async fn usage_uses_pi_context_snapshot_instead_of_cumulative_tokens() {
    let (_dir, mut session) = start("normal").await;
    session.enable_usage_reporting();
    session
        .send_message("usage-prompt", "hello", None)
        .await
        .unwrap();
    loop {
        if let Event::Usage {
            context: Some(context),
            weekly,
            ..
        } = next(&mut session).await
        {
            assert_eq!(context.used_tokens, 84000);
            assert_eq!(context.window_tokens, 200000);
            assert!(weekly.is_none());
            break;
        }
    }
    session.stop().await.unwrap();
}
