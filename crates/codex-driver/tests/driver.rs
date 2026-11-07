use boblm_codex_driver::{CodexOptions, CodexSession, McpEndpoint};
use boblm_protocol::{Event, LifecycleState, Priority};
use std::{path::Path, time::Duration};
use tokio::{process::Command, time::timeout};

const DEADLINE: Duration = Duration::from_secs(5);

fn fake(envs: &[(&str, &str)]) -> Command {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut command = Command::new("python3");
    command.arg("-u").arg(root.join("tests/fake_app_server.py"));
    for (key, value) in envs {
        command.env(key, value);
    }
    command
}
fn options(resume: Option<&'static str>) -> CodexOptions<'static> {
    CodexOptions {
        cwd: Path::new(env!("CARGO_MANIFEST_DIR")),
        resume,
        model: "gpt-6-luna",
        effort: Some("low"),
        mcp: None,
        home_lock: None,
    }
}
async fn start() -> CodexSession {
    CodexSession::spawn_command(fake(&[]), options(None))
        .await
        .unwrap()
}
/// The next event, without the `turn_started` progress notice that opens every turn.
async fn next(session: &mut CodexSession) -> Event {
    loop {
        let event = timeout(DEADLINE, session.events.recv())
            .await
            .expect("an event in time")
            .expect("the stream is open");
        if !matches!(&event, Event::System { subtype: Some(s), .. } if s == "turn_started") {
            return event;
        }
    }
}
/// Events up to and including the next result.
async fn until_result(session: &mut CodexSession) -> Vec<Event> {
    let mut events = Vec::new();
    loop {
        let event = next(session).await;
        let done = matches!(event, Event::Result { .. });
        events.push(event);
        if done {
            return events;
        }
    }
}
fn started(events: &[Event], uuid: &str) -> bool {
    events.iter().any(|e| matches!(e, Event::CommandLifecycle { command_uuid, state: LifecycleState::Started } if command_uuid == uuid))
}
fn answer(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Assistant { message } => {
                message["content"][0]["text"].as_str().map(str::to_owned)
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("|")
}

#[tokio::test]
async fn the_api_home_child_sees_the_key_in_its_environment_only() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("env.log");
    boblm_codex_driver::account::prepare_api_home(temp.path()).unwrap();
    let mut command = fake(&[("FAKE_LOG", log.to_str().unwrap())]);
    command
        .env("CODEX_HOME", temp.path())
        .env("OPENAI_API_KEY", "fictional-openai-key");
    let session = CodexSession::spawn_command(command, options(None))
        .await
        .unwrap();
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("openai_key_present=True"), "{text}");
    assert!(!text.contains("fictional-openai-key"), "{text}");
    assert!(!temp.path().join("auth.json").exists());
    session.stop().await.unwrap();
}

#[tokio::test]
async fn the_fake_child_sees_the_codex_home_from_its_command() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("env.log");
    let mut command = fake(&[("FAKE_LOG", log.to_str().unwrap())]);
    command.env("CODEX_HOME", temp.path());
    let session = CodexSession::spawn_command(command, options(None))
        .await
        .unwrap();
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(
        text.contains(&format!("codex_home={}", temp.path().display())),
        "{text}"
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn concurrent_first_use_of_a_fresh_codex_home_is_serialized() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    boblm_codex_driver::account::prepare_api_home(&home).unwrap();
    let log = temp.path().join("serial.log");
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let home = home.clone();
        let log = log.clone();
        tasks.push(tokio::spawn(async move {
            let mut command = fake(&[
                ("FAKE_SERIAL_LOG", log.to_str().unwrap()),
                ("FAKE_INIT_DELAY_MS", "100"),
            ]);
            command
                .env("CODEX_HOME", &home)
                .env("OPENAI_API_KEY", "fictional-openai-key");
            CodexSession::spawn_command(
                command,
                CodexOptions {
                    cwd: Path::new(env!("CARGO_MANIFEST_DIR")),
                    resume: None,
                    model: "gpt-6-luna",
                    effort: Some("low"),
                    mcp: None,
                    home_lock: Some(&home),
                },
            )
            .await
        }));
    }
    let mut sessions = Vec::new();
    for task in tasks {
        sessions.push(task.await.unwrap().unwrap());
    }
    let log = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<_> = log.lines().collect();
    assert_eq!(lines.len(), 8, "{log}");
    for pair in lines.chunks(2) {
        assert!(pair[0].starts_with("start "), "{log}");
        assert!(pair[1].starts_with("ready "), "{log}");
    }
    for session in sessions {
        session.stop().await.unwrap();
    }
}

#[tokio::test]
async fn a_new_chat_announces_its_conversation_and_a_message_is_consumed_under_its_own_id() {
    let mut session = start().await;
    let Event::System {
        subtype,
        session_id,
        ..
    } = next(&mut session).await
    else {
        panic!("expected init")
    };
    assert_eq!(subtype.as_deref(), Some("init"));
    assert_eq!(session_id, session.session_id());
    assert!(session_id.unwrap().starts_with("thread-"));
    session.send_message("m-1", "hello", None).await.unwrap();
    let events = until_result(&mut session).await;
    assert!(
        started(&events, "m-1"),
        "Codex announcing our ID is the proof of consumption: {events:?}"
    );
    assert_eq!(answer(&events), "echo: hello [model=gpt-6-luna effort=low]");
    assert!(events.iter().any(
        |e| matches!(e, Event::StreamEvent { event } if event["type"] == "content_block_delta")
    ));
    assert!(
        matches!(events.last(), Some(Event::Result { subtype: Some(s), is_error: Some(false) }) if s == "success")
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn a_chosen_model_and_effort_apply_from_the_next_turn_without_a_restart() {
    let mut session = start().await;
    next(&mut session).await;
    session.configure("gpt-6.1-sol", Some("high"));
    session.send_message("m-1", "again", None).await.unwrap();
    assert_eq!(
        answer(&until_result(&mut session).await),
        "echo: again [model=gpt-6.1-sol effort=high]"
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn a_message_sent_during_a_turn_steers_it_and_a_late_steer_starts_a_turn() {
    let mut session = start().await;
    next(&mut session).await;
    session.send_message("m-1", "slow", None).await.unwrap();
    assert!(matches!(
        next(&mut session).await,
        Event::CommandLifecycle {
            state: LifecycleState::Started,
            ..
        }
    ));
    session
        .send_message("m-2", "change course", Some(Priority::Now))
        .await
        .unwrap();
    let events = until_result(&mut session).await;
    assert!(started(&events, "m-2"));
    assert_eq!(answer(&events), "steered: change course");
    // The turn is over: a "steer" now finds nothing to steer and simply starts a turn.
    session
        .send_message("m-3", "after", Some(Priority::Now))
        .await
        .unwrap();
    let events = until_result(&mut session).await;
    assert!(started(&events, "m-3"));
    assert_eq!(answer(&events), "echo: after [model=gpt-6-luna effort=low]");
    session.stop().await.unwrap();
}

#[tokio::test]
async fn a_steer_that_finds_its_turn_already_gone_starts_a_new_turn_instead_of_failing() {
    let mut session = start().await;
    next(&mut session).await;
    session.send_message("m-1", "ghost", None).await.unwrap();
    // The driver still believes a turn is running; Codex says there is none.
    session
        .send_message("m-2", "carry on", Some(Priority::Now))
        .await
        .unwrap();
    let events = until_result(&mut session).await;
    assert!(started(&events, "m-2"), "{events:?}");
    assert_eq!(
        answer(&events),
        "echo: carry on [model=gpt-6-luna effort=low]"
    );
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::CommandLifecycle {
            state: LifecycleState::Refused,
            ..
        }
    )));
    session.stop().await.unwrap();
}

#[tokio::test]
async fn stopping_a_turn_ends_it_with_an_interrupted_result() {
    let mut session = start().await;
    next(&mut session).await;
    session.send_message("m-1", "slow", None).await.unwrap();
    next(&mut session).await; // consumed
    session.request_interrupt().await.unwrap();
    let events = until_result(&mut session).await;
    assert!(
        matches!(events.last(), Some(Event::Result { subtype: Some(s), is_error: Some(false) }) if s == "interrupted"),
        "{events:?}"
    );
    // With nothing running there is nothing to stop.
    session.request_interrupt().await.unwrap();
    session.stop().await.unwrap();
}

#[tokio::test]
async fn codex_refusing_a_message_says_why_and_rejects_it() {
    let mut session = start().await;
    next(&mut session).await;
    session.configure("bad-model", None);
    session.send_message("m-1", "hi", None).await.unwrap();
    let Event::Assistant { message } = next(&mut session).await else {
        panic!("expected the reason first")
    };
    let text = message["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Codex did not accept this message")
            && text.contains("unknown model bad-model"),
        "{text}"
    );
    assert!(
        matches!(next(&mut session).await, Event::CommandLifecycle { command_uuid, state: LifecycleState::Refused } if command_uuid == "m-1")
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn a_failed_turn_shows_its_error_and_ends_in_an_error_result() {
    let mut session = start().await;
    next(&mut session).await;
    session.send_message("m-1", "fail", None).await.unwrap();
    let events = until_result(&mut session).await;
    assert!(
        answer(&events).contains("Codex stopped this turn with an error: model overloaded."),
        "{events:?}"
    );
    assert!(
        matches!(events.last(), Some(Event::Result { subtype: Some(s), is_error: Some(true) }) if s == "error_during_execution")
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn shell_commands_show_as_a_tool_call_and_its_output() {
    let mut session = start().await;
    next(&mut session).await;
    session.send_message("m-1", "tool", None).await.unwrap();
    let events = until_result(&mut session).await;
    assert!(events.iter().any(|e| matches!(e, Event::Assistant { message } if message["content"][0]["name"] == "shell" && message["content"][0]["input"]["command"] == "echo hi")));
    assert!(events.iter().any(
        |e| matches!(e, Event::User { message } if message["content"][0]["content"] == "hi\n")
    ));
    session.stop().await.unwrap();
}

#[tokio::test]
async fn an_accepted_compact_request_alone_is_not_the_start_of_the_command() {
    let mut session =
        CodexSession::spawn_command(fake(&[("FAKE_COMPACT_SILENT", "1")]), options(None))
            .await
            .unwrap();
    next(&mut session).await;
    session.send_message("c-1", "/compact", None).await.unwrap();
    let mut events = Vec::new();
    while let Ok(Some(event)) = timeout(DEADLINE, session.events.recv()).await {
        events.push(event);
    }
    assert!(
        !started(&events, "c-1"),
        "the request reply is not proof Codex compacted anything: {events:?}"
    );
}

#[tokio::test]
async fn compact_and_clear_map_to_codex_compaction_and_a_new_conversation() {
    let mut session = start().await;
    let Event::System {
        session_id: first, ..
    } = next(&mut session).await
    else {
        panic!()
    };
    session.send_message("c-1", "/compact", None).await.unwrap();
    let events = until_result(&mut session).await;
    assert!(started(&events, "c-1"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::System { subtype:Some(s), .. } if s == "compact_started"))
    );
    assert!(
        events.iter().any(
            |e| matches!(e, Event::System { subtype: Some(s), .. } if s == "compact_boundary")
        ),
        "{events:?}"
    );
    session.send_message("c-2", "/clear", None).await.unwrap();
    let events = until_result(&mut session).await;
    assert!(started(&events, "c-2"));
    let second = events.iter().find_map(|e| match e {
        Event::System {
            subtype: Some(s),
            session_id,
            ..
        } if s == "init" => session_id.clone(),
        _ => None,
    });
    assert!(
        second.is_some() && second != first,
        "a new conversation ID is reported: {events:?}"
    );
    assert_eq!(second, session.session_id());
    assert!(
        matches!(next(&mut session).await, Event::CommandLifecycle { command_uuid, state: LifecycleState::Completed } if command_uuid == "c-2")
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn a_chat_resumes_its_conversation_and_a_failed_resume_never_starts_a_new_one() {
    let mut session = CodexSession::spawn_command(fake(&[]), options(Some("thread-old")))
        .await
        .unwrap();
    assert_eq!(session.session_id().as_deref(), Some("thread-old"));
    let Event::System { session_id, .. } = next(&mut session).await else {
        panic!()
    };
    assert_eq!(session_id.as_deref(), Some("thread-old"));
    session.stop().await.unwrap();
    let error = CodexSession::spawn_command(
        fake(&[("FAKE_RESUME_FAILS", "1")]),
        options(Some("thread-old")),
    )
    .await
    .err()
    .expect("a failed resume is an error");
    let text = error.to_string();
    assert!(
        text.contains("could not resume its conversation thread-old")
            && text.contains("no new conversation was started"),
        "{text}"
    );
}

#[tokio::test]
async fn startup_problems_are_explained() {
    let dead = CodexSession::spawn_command(fake(&[("FAKE_EXIT_AT_START", "1")]), options(None))
        .await
        .err()
        .unwrap();
    assert!(
        dead.to_string()
            .contains("Codex exited before answering the initialize request"),
        "{dead}"
    );
    let missing = CodexSession::spawn_command(Command::new("/nonexistent/codex"), options(None))
        .await
        .err()
        .unwrap();
    let text = missing.to_string();
    assert!(
        text.contains("/nonexistent/codex") && text.contains("installed") && text.contains("PATH"),
        "{text}"
    );
}

#[tokio::test]
async fn peer_tools_are_configured_with_the_capability_in_the_environment_not_the_arguments() {
    let mcp = McpEndpoint {
        url: "http://127.0.0.1:3000/mcp/chat",
        token: "fake-capability",
    };
    let mut options = options(None);
    options.mcp = Some(mcp);
    // The fake checks the thread/start config, its environment and its own arguments, and logs
    // a verdict naming whatever was wrong.
    let log = tempfile::NamedTempFile::new_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target"))
        .unwrap();
    let session = CodexSession::spawn_command(
        fake(&[
            ("FAKE_EXPECT_MCP", "http://127.0.0.1:3000/mcp/chat"),
            ("FAKE_LOG", log.path().to_str().unwrap()),
        ]),
        options,
    )
    .await
    .unwrap();
    session.stop().await.unwrap();
    let logged = std::fs::read_to_string(log.path()).unwrap();
    assert!(logged.contains("mcp-ok"), "{logged}");
    assert!(!logged.contains("mcp-bad"), "{logged}");
}

#[tokio::test]
async fn requests_that_wait_for_the_owner_are_declined_and_a_stopped_stream_ends() {
    let log = tempfile::NamedTempFile::new_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target"))
        .unwrap();
    let mut session = CodexSession::spawn_command(
        fake(&[("FAKE_LOG", log.path().to_str().unwrap())]),
        options(None),
    )
    .await
    .unwrap();
    next(&mut session).await;
    session.send_message("m-1", "ask", None).await.unwrap();
    until_result(&mut session).await;
    assert!(
        std::fs::read_to_string(log.path())
            .unwrap()
            .contains("answered-900 error=True"),
        "the question must be declined, not left hanging"
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn codex_capacity_errors_are_visible_without_claiming_consumption_or_completion() {
    let mut session = start().await;
    next(&mut session).await;
    session
        .send_message("capacity-1", "capacity", None)
        .await
        .unwrap();
    let event = next(&mut session).await;
    let text = answer(&[event]);
    assert!(
        text.contains("model is at capacity") && text.contains("retry"),
        "{text}"
    );
    assert!(
        timeout(Duration::from_millis(100), session.events.recv())
            .await
            .is_err()
    );
    session.stop().await.unwrap();
}

// Run the real driver in a subprocess so stderr is observed at the server's logging boundary.
#[tokio::test]
async fn codex_diagnostic_probe() {
    if std::env::var_os("BOBLM_DIAGNOSTIC_PROBE").is_none() {
        return;
    }
    let mut opts = options(None);
    opts.mcp = Some(McpEndpoint {
        url: "http://localhost/mcp",
        token: "fake-capability",
    });
    let limit = boblm_tuning::MAX_AGENT_DIAGNOSTIC_BYTES.to_string();
    let mut session = CodexSession::spawn_command(fake(&[("FAKE_DIAGNOSTIC_LIMIT", &limit)]), opts)
        .await
        .unwrap();
    next(&mut session).await;
    session
        .send_message("diagnostic-1", "diagnostics", None)
        .await
        .unwrap();
    timeout(DEADLINE, async {
        while session.events.recv().await.is_some() {}
    })
    .await
    .unwrap();
    session.stop().await.unwrap();
}

#[test]
fn codex_diagnostics_reach_the_server_log_with_context_and_without_the_capability() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "codex_diagnostic_probe", "--nocapture"])
        .env("BOBLM_DIAGNOSTIC_PROBE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(
        log.contains("upstream unavailable") && log.contains("[REDACTED]"),
        "{log}"
    );
    assert!(!log.contains("fake-capability"), "{log}");
    assert!(!log.contains("oversized-secret-"), "{log}");
    assert!(
        log.contains("exceeded the diagnostic limit")
            && log.contains("final diagnostic without newline"),
        "{log}"
    );
    assert!(
        log.contains("provider connection failed") && log.contains("willRetry=false"),
        "{log}"
    );
    assert!(
        log.contains("pid=") && log.contains("thread=thread-"),
        "{log}"
    );
    assert!(
        log.contains("invalid JSON") && log.contains("exit status: 7"),
        "{log}"
    );
}

#[tokio::test]
async fn usage_reports_the_parent_context_and_weekly_window_without_starting_a_turn() {
    let mut session = start().await;
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
    assert_eq!(
        context.unwrap().used_tokens,
        84000,
        "child thread usage was ignored"
    );
    assert_eq!(
        weekly.unwrap().used_percent,
        37.0,
        "weekly usage uses the weekly window"
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn token_usage_ignores_child_threads_and_labels_the_parent_model() {
    let mut session = start().await;
    session.enable_usage_reporting();
    let parent = loop {
        if let Event::TokenUsage {
            model,
            input_tokens,
            cached_input_tokens,
            cache_write_input_tokens,
            output_tokens,
        } = next(&mut session).await
        {
            assert_ne!(
                input_tokens, 999,
                "child thread token usage reached the parent"
            );
            break (
                model,
                input_tokens,
                cached_input_tokens,
                cache_write_input_tokens,
                output_tokens,
            );
        }
    };
    assert_eq!(parent, ("gpt-6-luna".into(), 76000, 4000, 0, 100));
    session.stop().await.unwrap();
}

#[tokio::test]
async fn background_jobs_survive_turn_completion_and_inventory_failures() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("jobs.json");
    std::fs::write(&path, r#"{"parent":[{"processId":"1","command":"bin/land"}],"child":[{"processId":"2","command":"cargo test"}]}"#).unwrap();
    let mut session = CodexSession::spawn_command(
        fake(&[("FAKE_JOBS", path.to_str().unwrap())]),
        options(None),
    )
    .await
    .unwrap();
    session
        .send_message("background", "hello", None)
        .await
        .unwrap();
    until_result(&mut session).await;
    session.enable_background_reporting();
    let Event::BackgroundJobs { snapshot } = next(&mut session).await else {
        panic!("inventory")
    };
    assert!(snapshot.available);
    assert_eq!(snapshot.tasks.len(), 2);
    assert!(
        snapshot
            .tasks
            .iter()
            .any(|j| j.command == "bin/land" && j.agent_key.is_none())
    );
    assert!(
        snapshot
            .tasks
            .iter()
            .any(|j| j.command == "cargo test" && j.agent_key.as_deref() == Some("child-thread"))
    );
    std::fs::write(&path, "invalid").unwrap();
    let Event::BackgroundJobs { snapshot } = next(&mut session).await else {
        panic!("unavailable inventory")
    };
    assert!(!snapshot.available);
    assert_eq!(
        snapshot.tasks.len(),
        2,
        "a failed read must not forget running jobs"
    );
    std::fs::write(&path, "{}").unwrap();
    let Event::BackgroundJobs { snapshot } = next(&mut session).await else {
        panic!("completed inventory")
    };
    assert!(snapshot.available);
    assert!(snapshot.tasks.is_empty());
    session.stop().await.unwrap();
}

#[tokio::test]
async fn stale_steering_refreshes_the_current_turn_and_delivers_once() {
    let mut session = start().await;
    next(&mut session).await;
    session
        .send_message("m-1", "replaced-turn", None)
        .await
        .unwrap();
    session
        .send_message("m-2", "carry on", Some(Priority::Now))
        .await
        .unwrap();
    let mut events = Vec::new();
    loop {
        let event = next(&mut session).await;
        let done = matches!(
            event,
            Event::Result { .. }
                | Event::CommandLifecycle {
                    state: LifecycleState::Refused,
                    ..
                }
        );
        events.push(event);
        if done {
            break;
        }
    }
    assert!(started(&events, "m-2"), "{events:?}");
    assert_eq!(answer(&events), "steered: carry on");
    assert_eq!(events.iter().filter(|e| matches!(e, Event::CommandLifecycle { command_uuid, state: LifecycleState::Started } if command_uuid == "m-2")).count(), 1);
    session.stop().await.unwrap();
}

#[tokio::test]
async fn stale_steering_stops_after_one_retry_or_a_failed_refresh() {
    for env in [
        "FAKE_TURN_READ_FAILS",
        "FAKE_TURN_READ_INVALID",
        "FAKE_TURN_CHANGES_AGAIN",
    ] {
        let mut session = CodexSession::spawn_command(fake(&[(env, "1")]), options(None))
            .await
            .unwrap();
        next(&mut session).await;
        session
            .send_message("m-1", "replaced-turn", None)
            .await
            .unwrap();
        timeout(
            DEADLINE,
            session.send_message("m-2", "carry on", Some(Priority::Now)),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            matches!(next(&mut session).await, Event::Assistant { .. }),
            "{env}"
        );
        assert!(
            matches!(
                next(&mut session).await,
                Event::CommandLifecycle {
                    state: LifecycleState::Refused,
                    ..
                }
            ),
            "{env}"
        );
        session.stop().await.unwrap();
    }
}

#[tokio::test]
async fn stale_steering_starts_a_turn_if_the_current_one_ended_during_refresh() {
    let mut session =
        CodexSession::spawn_command(fake(&[("FAKE_TURN_ENDS_BEFORE_READ", "1")]), options(None))
            .await
            .unwrap();
    next(&mut session).await;
    session
        .send_message("m-1", "replaced-turn", None)
        .await
        .unwrap();
    session
        .send_message("m-2", "carry on", Some(Priority::Now))
        .await
        .unwrap();
    let events = until_result(&mut session).await;
    assert!(started(&events, "m-2"), "{events:?}");
    assert_eq!(
        answer(&events),
        "echo: carry on [model=gpt-6-luna effort=low]"
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn child_turn_notifications_do_not_replace_or_finish_the_parent_turn() {
    let mut session =
        CodexSession::spawn_command(fake(&[("FAKE_TURN_READ_FAILS", "1")]), options(None))
            .await
            .unwrap();
    next(&mut session).await;
    session
        .send_message("m-1", "child-turn", None)
        .await
        .unwrap();
    assert!(matches!(
        next(&mut session).await,
        Event::CommandLifecycle {
            state: LifecycleState::Started,
            ..
        }
    ));
    session
        .send_message("m-2", "carry on", Some(Priority::Now))
        .await
        .unwrap();
    let events = until_result(&mut session).await;
    assert!(started(&events, "m-2"), "{events:?}");
    assert_eq!(answer(&events), "steered: carry on");
    session.stop().await.unwrap();
}
