use boblm_store::Store;
use boblm_tuning::{ExhaustedStep, StepId};

#[test]
fn role_exhaustion_marks_are_replaced_and_old_sql_still_runs() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("role-exhaustion.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("root", "/fake", 1).unwrap();
    store
        .mark_role_step_exhausted(
            StepId::ReviewerOpenAiApi,
            2_000_000_000,
            1_900_000_000,
            "fixture",
        )
        .unwrap();
    assert_eq!(
        store.reader().unwrap().role_exhaustions().unwrap(),
        vec![ExhaustedStep {
            step: StepId::ReviewerOpenAiApi,
            until: 2_000_000_000,
        }]
    );
    store
        .mark_role_step_exhausted(
            StepId::ReviewerOpenAiApi,
            2_100_000_000,
            2_000_000_000,
            "replaced",
        )
        .unwrap();
    assert_eq!(
        store.reader().unwrap().role_exhaustions().unwrap(),
        vec![ExhaustedStep {
            step: StepId::ReviewerOpenAiApi,
            until: 2_100_000_000,
        }]
    );

    store
        .mark_role_step_exhausted(
            StepId::ReviewerOpenAiApi,
            2_000_000_120,
            2_000_000_000,
            "TPM cooldown",
        )
        .unwrap();
    assert_eq!(
        store.reader().unwrap().role_exhaustions().unwrap()[0].until,
        2_100_000_000,
        "a cooldown must not shorten a quota or billing mark"
    );

    // The previous release's sessions SQL still runs after migration 037.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let fixture = include_str!("fixtures/sessions_a3fb372.sql");
    let sql = fixture
        .split("-- func: ")
        .find_map(|block| {
            let mut lines = block.lines();
            (lines.next()?.trim() == "list").then(|| {
                lines
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .trim_end_matches(';')
                    .to_owned()
            })
        })
        .unwrap();
    let mut statement = conn.prepare(&sql).unwrap();
    let mut rows = statement.query([]).unwrap();
    let row = rows
        .next()
        .unwrap()
        .expect("the old list query must return a row");
    let id: String = row.get(0).unwrap();
    assert_eq!(id, "root");
}
