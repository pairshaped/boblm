use boblm_store::{PeerSpawn, Store};
use boblm_tuning::*;

#[allow(clippy::too_many_arguments)]
fn request(
    project: &str,
    id: &str,
    root: &str,
    agent: &str,
    model: &str,
    account: Option<&str>,
    hidden: bool,
) -> PeerSpawn {
    PeerSpawn {
        request_id: id.into(),
        caller: root.into(),
        arguments: serde_json::json!({"id":id}).to_string(),
        project_id: project.into(),
        agent: agent.into(),
        model: model.into(),
        effort: "high".into(),
        title: "Work".into(),
        content: "work".into(),
        created_at: 1000,
        parent_session_id: hidden.then(|| root.into()),
        root_session_id: hidden.then(|| root.into()),
        role: Some(
            if id.ends_with('0') {
                "planner"
            } else {
                "implementer"
            }
            .into(),
        ),
        resolved: None,
        working_directory: None,
        account_key: account.map(str::to_owned),
    }
}

#[test]
fn provider_caps_span_roots_include_owner_chats_and_preserve_retries() {
    for (agent, model, account, cap, group) in [
        (
            "codex",
            MODEL_GPT_6_LUNA,
            Some(CODEX_ACCOUNT_KEY_API),
            OPENAI_API_MAX_ACTIVE_REVIEWERS,
            ProviderGroup::LunaApi,
        ),
        (
            "claude",
            MODEL_HAIKU,
            Some(CLAUDE_ACCOUNT_KEY_API_A),
            MAX_OPEN_HAIKU_API_AGENTS,
            ProviderGroup::HaikuApiA,
        ),
        (
            "claude",
            MODEL_HAIKU,
            Some(CLAUDE_ACCOUNT_KEY_API_B),
            MAX_OPEN_HAIKU_API_AGENTS,
            ProviderGroup::HaikuApiB,
        ),
        (
            "claude",
            MODEL_OPUS,
            Some(CLAUDE_ACCOUNT_KEY_PLAN_B),
            MAX_OPEN_CLAUDE_PLAN_AGENTS,
            ProviderGroup::ClaudePlanB,
        ),
        (
            "codex",
            MODEL_GPT_6_1_SOL,
            Some(CODEX_ACCOUNT_KEY_PLAN),
            MAX_OPEN_CODEX_PLAN_AGENTS,
            ProviderGroup::CodexPlan,
        ),
        (
            "claude",
            MODEL_SONNET,
            Some(CLAUDE_ACCOUNT_KEY_PLAN_A),
            MAX_OPEN_CLAUDE_PLAN_AGENTS,
            ProviderGroup::ClaudePlanA,
        ),
        (
            "pi",
            MODEL_DEEPSEEK_V41_FLASH,
            None,
            MAX_OPEN_DSF_API_AGENTS,
            ProviderGroup::DsfApi,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("caps.sqlite")).unwrap();
        // Unrecognized provider roots do not consume any tested group.
        let project = store
            .add_project("project", "/fake", "Project", 1)
            .unwrap()
            .0;
        for root in ["a", "b"] {
            store
                .create_session_in(boblm_store::NewSession {
                    id: root,
                    project_id: &project.id,
                    name: "Root",
                    agent: "pi",
                    model: "other",
                    effort: None,
                    created_at: 1,
                })
                .unwrap();
        }
        for index in 0..cap {
            let id = format!("child{index}");
            store
                .spawn_peer(request(
                    &project.id,
                    &id,
                    if index % 2 == 0 { "a" } else { "b" },
                    agent,
                    model,
                    account,
                    index % 2 == 0,
                ))
                .unwrap();
        }
        assert!(
            store
                .reader()
                .unwrap()
                .open_provider_counts()
                .unwrap()
                .rejection(group)
                .is_some()
        );
        assert!(
            !store
                .spawn_peer(request(
                    &project.id,
                    "child0",
                    "a",
                    agent,
                    model,
                    account,
                    true
                ))
                .unwrap()
        );
        let error = store
            .spawn_peer(request(
                &project.id,
                "over",
                "b",
                agent,
                model,
                account,
                false,
            ))
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("{cap} of {cap}")), "{error}");
        assert!(error.contains("owner chats"), "{error}");
        store.set_archived("child0", true).unwrap();
        store
            .spawn_peer(request(
                &project.id,
                "replacement",
                "a",
                agent,
                model,
                account,
                true,
            ))
            .unwrap();
        // Owner operations remain allowed even above the cap.
        store
            .create_session_in(boblm_store::NewSession {
                id: "owner",
                project_id: &project.id,
                name: "Owner",
                agent,
                model,
                effort: None,
                created_at: 2,
            })
            .unwrap();
        if let Some(key) = account {
            store.set_claude_account_key("owner", key).unwrap();
        }
        store.set_archived("owner", true).unwrap();
        store.set_archived("owner", false).unwrap();
        store.set_archived("child0", false).unwrap();
        let count_reason = store
            .reader()
            .unwrap()
            .open_provider_counts()
            .unwrap()
            .rejection(group)
            .unwrap();
        assert!(
            count_reason.contains(&format!("{} of {cap}", cap + 2)),
            "{count_reason}"
        );
        // Classification reads the current choice, regardless of the original receipt.
        if agent == "pi" {
            store
                .set_model_effort("owner", "another-model", None)
                .unwrap();
        } else {
            store
                .set_claude_account_key("owner", "unrecognized-account")
                .unwrap();
        }
        let count_reason = store
            .reader()
            .unwrap()
            .open_provider_counts()
            .unwrap()
            .rejection(group)
            .unwrap();
        assert!(
            count_reason.contains(&format!("{} of {cap}", cap + 1)),
            "{count_reason}"
        );
    }
}

#[test]
fn provider_caps_serialize_concurrent_spawns_for_the_last_slot() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("concurrent.sqlite")).unwrap();
    store.create_session("root", "/fake", 1).unwrap();
    let project = store.reader().unwrap().sessions().unwrap()[0]
        .project_id
        .clone()
        .unwrap();
    for index in 0..MAX_OPEN_DSF_API_AGENTS - 1 {
        store
            .spawn_peer(request(
                &project,
                &format!("seed{index}"),
                "root",
                "pi",
                MODEL_DEEPSEEK_V41_FLASH,
                None,
                false,
            ))
            .unwrap();
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|index| {
            let store = store.clone();
            let project = project.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.spawn_peer(request(
                    &project,
                    &format!("race{index}"),
                    "root",
                    "pi",
                    MODEL_DEEPSEEK_V41_FLASH,
                    None,
                    false,
                ))
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        store
            .reader()
            .unwrap()
            .open_provider_counts()
            .unwrap()
            .count(ProviderGroup::DsfApi),
        MAX_OPEN_DSF_API_AGENTS
    );
}

#[test]
fn provider_caps_reviewers_have_separate_slots_and_archiving_releases_them() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("reviewers.sqlite")).unwrap();
    store.create_session("root", "/fake", 1).unwrap();
    let project = store.reader().unwrap().sessions().unwrap()[0]
        .project_id
        .clone()
        .unwrap();
    let make_reviewer = |id: &str, agent, model, account| {
        let mut spawn = request(&project, id, "root", agent, model, account, true);
        spawn.role = Some("reviewer".into());
        spawn
    };
    for index in 0..MAX_OPEN_REVIEWERS_PER_PROVIDER {
        store
            .spawn_peer(make_reviewer(
                &format!("review{index}"),
                "codex",
                MODEL_GPT_6_LUNA,
                Some(CODEX_ACCOUNT_KEY_PLAN),
            ))
            .unwrap();
    }
    assert!(
        !store
            .spawn_peer(make_reviewer(
                "review0",
                "codex",
                MODEL_GPT_6_LUNA,
                Some(CODEX_ACCOUNT_KEY_PLAN)
            ))
            .unwrap()
    );
    let error = store
        .spawn_peer(make_reviewer(
            "over",
            "codex",
            MODEL_GPT_6_LUNA,
            Some(CODEX_ACCOUNT_KEY_PLAN),
        ))
        .unwrap_err()
        .to_string();
    assert!(error.contains("reviewers open"), "{error}");
    store
        .spawn_peer(request(
            &project,
            "implement",
            "root",
            "codex",
            MODEL_GPT_6_1_SOL,
            Some(CODEX_ACCOUNT_KEY_PLAN),
            false,
        ))
        .unwrap();
    let counts = store.reader().unwrap().open_provider_counts().unwrap();
    assert_eq!(
        counts.count(ProviderGroup::CodexPlan),
        MAX_OPEN_REVIEWERS_PER_PROVIDER + 1
    );
    store
        .spawn_peer(make_reviewer("other", "pi", MODEL_DEEPSEEK_V41_FLASH, None))
        .unwrap();
    store.set_archived("review0", true).unwrap();
    store
        .spawn_peer(make_reviewer(
            "replacement",
            "codex",
            MODEL_GPT_6_LUNA,
            Some(CODEX_ACCOUNT_KEY_PLAN),
        ))
        .unwrap();
}

#[test]
fn provider_caps_claude_accounts_have_independent_reviewer_slots() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("account-reviewers.sqlite")).unwrap();
    store.create_session("root", "/fake", 1).unwrap();
    let project = store.reader().unwrap().sessions().unwrap()[0]
        .project_id
        .clone()
        .unwrap();
    for account in [CLAUDE_ACCOUNT_KEY_PLAN_A, CLAUDE_ACCOUNT_KEY_PLAN_B] {
        for index in 0..MAX_OPEN_REVIEWERS_PER_PROVIDER {
            let mut spawn = request(
                &project,
                &format!("{account}-{index}"),
                "root",
                "claude",
                MODEL_OPUS,
                Some(account),
                true,
            );
            spawn.role = Some("reviewer".into());
            store.spawn_peer(spawn).unwrap();
        }
        let mut over = request(
            &project,
            &format!("{account}-over"),
            "root",
            "claude",
            MODEL_OPUS,
            Some(account),
            true,
        );
        over.role = Some("reviewer".into());
        let error = store.spawn_peer(over).unwrap_err().to_string();
        assert!(error.contains("reviewers open"), "{error}");
    }
    let counts = store.reader().unwrap().open_provider_counts().unwrap();
    assert_eq!(
        counts.count(ProviderGroup::ClaudePlanA),
        MAX_OPEN_REVIEWERS_PER_PROVIDER + 1
    );
    assert_eq!(
        counts.count(ProviderGroup::ClaudePlanB),
        MAX_OPEN_REVIEWERS_PER_PROVIDER
    );
}
