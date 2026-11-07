use boblm_claude_driver::{ClaudeSession, SteerOutcome};
use boblm_protocol::{Event, Priority};
use std::{path::Path, time::Duration};
use tokio::{process::Command, time::timeout};
const DEADLINE: Duration = Duration::from_secs(3);

async fn fake(resume: Option<&str>) -> ClaudeSession {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut command = Command::new("python3");
    command.arg("-u").arg(root.join("tests/fake_child.py"));
    if let Some(id) = resume {
        command.env("EXPECT_RESUME", id);
    }
    ClaudeSession::spawn_command(command, root, resume)
        .await
        .unwrap()
}
async fn next(session: &mut ClaudeSession) -> Event {
    timeout(DEADLINE, session.events.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn lifecycle_ack_rejections_and_cutover() {
    let mut session = fake(None).await;
    assert!(matches!(next(&mut session).await, Event::System { .. }));
    for mode in ["started", "cutover", "unknown"] {
        assert_eq!(
            session
                .dispatch(mode, mode, Some(Priority::Now), DEADLINE)
                .await,
            SteerOutcome::Started
        );
    }
    for state in ["cancelled", "discarded", "refused"] {
        assert_eq!(
            session.dispatch(state, state, None, DEADLINE).await,
            SteerOutcome::Rejected
        );
    }
    let mut saw_other = false;
    let mut saw_delta = false;
    while let Ok(event) = session.events.try_recv() {
        saw_other |= matches!(event, Event::Other);
        saw_delta |= matches!(event, Event::StreamEvent { .. });
    }
    assert!(saw_other && saw_delta);
    session.stop().await.unwrap();
}

#[tokio::test]
async fn completed_alone_proves_consumption() {
    let session = fake(None).await;
    assert_eq!(
        session
            .dispatch("completed", "completed-only", None, DEADLINE)
            .await,
        SteerOutcome::Started
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn completed_after_result_proves_consumption() {
    let mut session = fake(None).await;
    next(&mut session).await;
    assert_eq!(
        session
            .dispatch(
                "follow-up",
                "result-then-completed",
                Some(Priority::Now),
                DEADLINE
            )
            .await,
        SteerOutcome::Started
    );
    assert!(matches!(next(&mut session).await, Event::Result { .. }));
    assert!(matches!(
        next(&mut session).await,
        Event::CommandLifecycle {
            state: boblm_protocol::LifecycleState::Completed,
            ..
        }
    ));
    session.stop().await.unwrap();
}

#[tokio::test]
async fn exit_resolves_pending_even_with_inherited_stdout() {
    for mode in ["exit", "exit-with-descendant"] {
        let session = fake(None).await;
        let outcome = timeout(
            Duration::from_secs(1),
            session.dispatch("pending", mode, None, DEADLINE),
        )
        .await
        .unwrap();
        assert_eq!(outcome, SteerOutcome::Uncertain);
        session.stop().await.unwrap();
    }
}

#[tokio::test]
async fn resume_init_uuid_priority_and_abort_wire() {
    let mut session = fake(Some("resume-id")).await;
    next(&mut session).await;
    assert_eq!(session.session_id().as_deref(), Some("fake-session"));
    session
        .send_message("client-id", "wire", Some(Priority::Now))
        .await
        .unwrap();
    let Event::Assistant { message } = next(&mut session).await else {
        panic!("wire echo")
    };
    assert_eq!(message["uuid"], "client-id");
    assert_eq!(message["priority"], "now");
    let request_id = session.abort().await.unwrap();
    loop {
        if let Event::ControlResponse { response } = next(&mut session).await {
            assert_eq!(response["request_id"], request_id);
            break;
        }
    }
    session.stop().await.unwrap();
}

#[tokio::test]
async fn cancelled_dispatch_cleans_registration() {
    let session = fake(None).await;
    assert!(
        timeout(
            Duration::from_millis(100),
            session.dispatch("cancel", "hang", None, DEADLINE)
        )
        .await
        .is_err()
    );
    assert_eq!(
        session.dispatch("cancel", "started", None, DEADLINE).await,
        SteerOutcome::Started
    );
    session.stop().await.unwrap();
}

#[cfg(target_os = "linux")]
fn alive(pid: u64) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !stat
            .split(')')
            .nth(1)
            .unwrap_or("")
            .trim_start()
            .starts_with('Z')
    })
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn stop_and_drop_kill_the_process_group() {
    for stop in [true, false] {
        let mut session = fake(None).await;
        next(&mut session).await;
        session.send("tree").await.unwrap();
        let Event::Assistant { message } = next(&mut session).await else {
            panic!("tree pids")
        };
        let parent = message["parent"].as_u64().unwrap();
        let child = message["child"].as_u64().unwrap();
        assert!(alive(parent) && alive(child));
        if stop {
            session.stop().await.unwrap();
        } else {
            drop(session);
        }
        timeout(DEADLINE, async {
            while alive(parent) || alive(child) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn abort_settles_pending_as_uncertain_and_session_can_continue() {
    let mut session = fake(None).await;
    next(&mut session).await;
    let (outcome, request_id) = tokio::join!(
        session.dispatch("pending", "hang", Some(Priority::Now), DEADLINE),
        async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            session.abort().await.unwrap()
        }
    );
    assert_eq!(outcome, SteerOutcome::Uncertain);
    let Event::ControlResponse { response } = next(&mut session).await else {
        panic!("interrupt response")
    };
    assert_eq!(response["request_id"], request_id);
    assert_eq!(response["subtype"], "success");
    assert_eq!(
        session.dispatch("next", "started", None, DEADLINE).await,
        SteerOutcome::Started
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn cancelled_partial_write_stops_the_process() {
    let mut session = fake(None).await;
    next(&mut session).await;
    session.send("block-input").await.unwrap();
    assert!(matches!(next(&mut session).await, Event::Assistant { .. }));
    let content = "x".repeat(2 * 1024 * 1024);
    assert_eq!(
        session
            .dispatch("partial", &content, None, Duration::from_millis(50))
            .await,
        SteerOutcome::Uncertain
    );
    assert!(
        timeout(DEADLINE, session.events.recv())
            .await
            .unwrap()
            .is_none()
    );
    assert!(session.send("next").await.is_err());
    session.stop().await.unwrap();
}

#[tokio::test]
async fn interrupt_timeout_or_crash_stops_the_session() {
    for mode in ["hang", "exit"] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut command = Command::new("python3");
        command
            .arg("-u")
            .arg(root.join("tests/fake_child.py"))
            .env("FAKE_INTERRUPT", mode);
        let mut session = ClaudeSession::spawn_command(command, root, None)
            .await
            .unwrap();
        next(&mut session).await;
        let error = session
            .abort_with_deadline(Duration::from_millis(100))
            .await
            .unwrap_err();
        assert_eq!(
            error.kind(),
            if mode == "hang" {
                std::io::ErrorKind::TimedOut
            } else {
                std::io::ErrorKind::BrokenPipe
            }
        );
        assert!(
            timeout(DEADLINE, session.events.recv())
                .await
                .unwrap()
                .is_none()
        );
        assert!(session.send("next").await.is_err());
        session.stop().await.unwrap();
    }
}

#[tokio::test]
async fn model_and_effort_controls_change_settings_without_replacing_session() {
    let mut session = fake(None).await;
    next(&mut session).await;
    let id = session.session_id();
    session.set_model(Some("opus")).await.unwrap();
    session.set_effort(Some("high")).await.unwrap();
    let settings = session
        .control(serde_json::json!({"subtype":"get_settings"}), DEADLINE)
        .await
        .unwrap();
    assert_eq!(settings["effective"]["model"], "opus");
    assert_eq!(settings["effective"]["effortLevel"], "high");
    session.set_model(None).await.unwrap();
    session.set_effort(None).await.unwrap();
    let settings = session
        .control(serde_json::json!({"subtype":"get_settings"}), DEADLINE)
        .await
        .unwrap();
    assert_eq!(settings["effective"]["model"], "default");
    assert!(settings["effective"]["effortLevel"].is_null());
    assert_eq!(session.session_id(), id);
    session.stop().await.unwrap();
}

#[tokio::test]
async fn http_mcp_is_passed_directly_to_the_child_through_a_private_file() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let runtime = tempfile::tempdir_in(root.join("../../target")).unwrap();
    let directory = runtime.path().join("mcp");
    let mut command = tokio::process::Command::new("python3");
    command
        .arg(root.join("tests/fake_child.py"))
        .env("EXPECT_MCP_URL", "http://127.0.0.1:3000/mcp/session");
    let mut session = ClaudeSession::spawn_command_with_mcp(
        command,
        root,
        None,
        "http://127.0.0.1:3000/mcp/session",
        "test-session-capability",
        &directory,
    )
    .await
    .unwrap();
    // The fake child asserts the file's modes and contents before it emits init.
    assert!(matches!(
        session.events.recv().await,
        Some(boblm_protocol::Event::System { .. })
    ));
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
    session.stop().await.unwrap();
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
}

#[tokio::test]
async fn stop_completes_when_the_child_exits_with_the_event_channel_full() {
    let session = fake(None).await;
    session.send("flood-exit").await.unwrap();
    // Leave every event unread so the reader parks on a full channel at exit.
    tokio::time::sleep(Duration::from_millis(500)).await;
    timeout(DEADLINE, session.stop())
        .await
        .expect("stop must not hang behind an unread event channel")
        .unwrap();
}

#[tokio::test]
async fn every_event_written_before_exit_reaches_a_slow_reader() {
    let mut session = fake(None).await;
    session.send("flood-exit").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut count = 0;
    let mut last = None;
    while let Some(event) = timeout(DEADLINE, session.events.recv()).await.unwrap() {
        count += 1;
        last = Some(event);
    }
    // The init event, 3000 numbered events and the final marker.
    assert_eq!(count, 3002);
    assert!(matches!(last, Some(Event::Assistant { .. })), "{last:?}");
    session.stop().await.unwrap();
}

#[tokio::test]
async fn invalid_utf8_and_oversized_lines_do_not_end_the_session() {
    let mut session = fake(None).await;
    assert_eq!(
        session.dispatch("garbage", "garbage", None, DEADLINE).await,
        SteerOutcome::Started
    );
    assert_eq!(
        session
            .dispatch("completed", "completed-only", None, DEADLINE)
            .await,
        SteerOutcome::Started
    );
    let mut other = 0;
    while let Ok(event) = session.events.try_recv() {
        other += usize::from(matches!(event, Event::Other));
    }
    assert_eq!(other, 1, "only the oversized line is reported, once");
    session.stop().await.unwrap();
}

#[tokio::test]
async fn a_burst_that_fits_the_pipe_is_not_lost_when_the_child_exits_first() {
    let mut session = fake(None).await;
    session.send("burst-exit").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut count = 0;
    while timeout(DEADLINE, session.events.recv())
        .await
        .unwrap()
        .is_some()
    {
        count += 1;
    }
    assert_eq!(count, 2001); // init plus the burst
}

#[tokio::test]
async fn stop_is_bounded_when_an_escaped_descendant_keeps_writing_after_exit() {
    let session = fake(None).await;
    session.send("escape-chatter").await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    timeout(Duration::from_secs(8), session.stop())
        .await
        .expect("stop must be bounded by the post-exit drain limit")
        .unwrap();
}

#[tokio::test]
async fn mcp_config_is_refused_in_a_runtime_directory_others_can_use() {
    use std::os::unix::fs::PermissionsExt;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runtime = tempfile::tempdir_in(root.join("../../target")).unwrap();
    std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let result = ClaudeSession::spawn_command_with_mcp(
        tokio::process::Command::new("python3"),
        root,
        None,
        "http://127.0.0.1:3000/mcp/session",
        "test-session-capability",
        runtime.path(),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(std::fs::read_dir(runtime.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_missing_claude_binary_is_explained() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let error =
        ClaudeSession::spawn_command(Command::new("/nonexistent/claude-binary"), root, None)
            .await
            .err()
            .expect("spawning a missing program must fail");
    let text = error.to_string();
    assert!(text.contains("/nonexistent/claude-binary"), "{text}");
    assert!(
        text.contains("installed") && text.contains("PATH"),
        "{text}"
    );
    assert!(text.contains("working directory"), "{text}");
}

#[tokio::test]
async fn a_control_request_that_never_answers_says_which_one_and_for_how_long() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut command = Command::new("python3");
    command
        .arg("-u")
        .arg(root.join("tests/fake_child.py"))
        .env("FAKE_INTERRUPT", "hang");
    let session = ClaudeSession::spawn_command(command, root, None)
        .await
        .unwrap();
    let error = session
        .abort_with_deadline(Duration::from_millis(300))
        .await
        .expect_err("a hung child never answers");
    let text = error.to_string();
    assert!(text.contains("interrupt request"), "{text}");
    assert!(text.contains("within 300 milliseconds"), "{text}");
    assert!(text.contains("stopped"), "{text}");
}

#[tokio::test]
async fn usage_controls_report_context_and_weekly_without_a_turn() {
    let mut session = fake(None).await;
    session.enable_usage_reporting();
    let mut context = None;
    let mut weekly = None;
    let mut five_hour = None;
    while context.is_none() || weekly.is_none() || five_hour.is_none() {
        if let Event::Usage {
            context: c,
            weekly: w,
            five_hour: f,
            ..
        } = next(&mut session).await
        {
            context = c.or(context);
            weekly = w.or(weekly);
            five_hour = f.or(five_hour);
        }
    }
    assert_eq!(five_hour.unwrap().used_percent, 80.0);
    assert_eq!(context.unwrap().used_tokens, 84000);
    assert_eq!(weekly.unwrap().used_percent, 37.0);
    assert_eq!(
        session
            .dispatch("after-usage", "started", None, DEADLINE)
            .await,
        SteerOutcome::Started
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn unanswered_usage_controls_do_not_block_delivery_or_kill_claude() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut command = Command::new("python3");
    command
        .arg("-u")
        .arg(root.join("tests/fake_child.py"))
        .env("FAKE_USAGE", "ignore");
    let mut session = ClaudeSession::spawn_command(command, root, None)
        .await
        .unwrap();
    session.enable_usage_reporting();
    next(&mut session).await; // Init enables the optional probes.
    assert_eq!(
        session
            .dispatch("during-usage", "started", None, DEADLINE)
            .await,
        SteerOutcome::Started
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn claude_steering_wire_labels_only_human_input() {
    let mut session = fake(None).await;
    next(&mut session).await;
    for (human, priority) in [(true, Priority::Now), (false, Priority::Next)] {
        session
            .send_message_with_origin("origin-test", "wire", Some(priority), human)
            .await
            .unwrap();
        let message = loop {
            if let Event::Assistant { message } = next(&mut session).await {
                break message;
            }
        };
        assert_eq!(message["priority"], if human { "now" } else { "next" });
        if human {
            assert_eq!(message["origin"], serde_json::json!({"kind":"human"}));
        } else {
            assert!(message.get("origin").is_none());
        }
    }
    session.stop().await.unwrap();
}

#[tokio::test]
async fn prompt_suggestion_tolerates_unknown_shapes() {
    let mut session = fake(None).await;
    assert!(matches!(next(&mut session).await, Event::System { .. }));
    session
        .send_message("prediction-test", "prompt-suggestion", None)
        .await
        .unwrap();
    assert!(
        matches!(next(&mut session).await, Event::PromptSuggestion { suggestion } if suggestion.is_object())
    );
    assert!(
        matches!(next(&mut session).await, Event::PromptSuggestion { suggestion } if suggestion.is_null())
    );
    assert!(matches!(next(&mut session).await, Event::Other));
    assert!(
        matches!(next(&mut session).await, Event::PromptSuggestion { suggestion } if suggestion == "Explain the example")
    );
    session.stop().await.unwrap();
}
