use async_trait::async_trait;
use boblm_protocol::{Event, LifecycleState, Priority};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::{
    app::App,
    schedule::{ScheduleArgs, format_rfc3339},
};
use boblm_store::{Session, Store};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};

const NOW: i64 = 1_800_000_000_000;
const MIN: i64 = 60_000;
const ID1: &str = "30000000-0000-4000-8000-000000000001";
const ID2: &str = "30000000-0000-4000-8000-000000000002";
const WORK: &str = "40000000-0000-4000-8000-000000000001";

struct Input {
    uuid: String,
    content: String,
    priority: Option<Priority>,
}
struct Fake {
    events: mpsc::UnboundedReceiver<Event>,
    inputs: mpsc::UnboundedSender<Input>,
}
#[async_trait]
impl Driver for Fake {
    async fn send(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
    ) -> io::Result<()> {
        self.inputs
            .send(Input {
                uuid: uuid.into(),
                content: content.into(),
                priority,
            })
            .map_err(io::Error::other)
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
    store: Store,
    _temp: tempfile::TempDir,
    app: Arc<App>,
    events: mpsc::UnboundedSender<Event>,
    inputs: mpsc::UnboundedReceiver<Input>,
}
async fn fixture() -> Fixture {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("schedules.sqlite")).unwrap();
    store.create_session("s", "/local", 1).unwrap();
    store.create_session("t", "/local", 1).unwrap();
    let (events_tx, events) = mpsc::unbounded_channel();
    let (inputs_tx, inputs) = mpsc::unbounded_channel();
    let factory = Arc::new(Factory(Mutex::new(Some(Fake {
        events,
        inputs: inputs_tx,
    }))));
    let app = App::open(store.clone(), factory, "schedule-test-token-only".into())
        .await
        .unwrap();
    Fixture {
        store,
        _temp: temp,
        app,
        events: events_tx,
        inputs,
    }
}
fn args(id: &str, patch: impl FnOnce(&mut ScheduleArgs)) -> ScheduleArgs {
    let mut args = ScheduleArgs {
        schedule_id: id.into(),
        prompt: "keep going".into(),
        at: None,
        in_minutes: None,
        every_minutes: None,
        end_at: None,
        max_runs: None,
    };
    patch(&mut args);
    args
}
impl Fixture {
    async fn next(&mut self) -> Input {
        tokio::time::timeout(Duration::from_secs(3), self.inputs.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn nothing(&mut self) {
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            self.inputs.try_recv().is_err(),
            "nothing should have been sent"
        );
    }
    async fn list(&self, chat: &str) -> Vec<serde_json::Value> {
        self.app.list_schedules(chat).await.unwrap()["schedules"]
            .as_array()
            .unwrap()
            .clone()
    }
    async fn idle(&self) {
        let handle = self.app.handle_for_tests("s").await;
        for _ in 0..200 {
            let status = handle.status().await.unwrap();
            if !status.turn_active && status.dispatching == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("chat never became idle");
    }
}

#[tokio::test]
async fn a_one_shot_fires_once_after_its_time_and_waits_for_idle() {
    let mut f = fixture().await;
    let created = f
        .app
        .schedule_prompt(
            "s",
            args(ID1, |a| a.at = Some(format_rfc3339(NOW + MIN))),
            NOW,
        )
        .await
        .unwrap();
    assert_eq!(created["kind"], "once");
    assert_eq!(created["state"], "active");
    assert_eq!(created["alreadyExisted"], false);
    assert_eq!(f.app.run_due_schedules(NOW + MIN - 1000).await.unwrap(), 0);
    f.nothing().await;
    assert_eq!(f.app.run_due_schedules(NOW + MIN + 1000).await.unwrap(), 1);
    let sent = f.next().await;
    assert_eq!(sent.content, "Scheduled prompt:\n\nkeep going");
    assert_eq!(sent.priority, None);
    assert_eq!(
        sent.uuid,
        boblm_server::schedule::message_id(ID1, parse(&created["nextRunAt"]))
    );
    let listed = &f.list("s").await[0];
    assert_eq!(
        (listed["state"].as_str(), listed["runs"].as_i64()),
        (Some("done"), Some(1))
    );
    assert_eq!(listed["nextRunAt"], serde_json::Value::Null);
    assert_eq!(
        f.app.run_due_schedules(NOW + 10 * MIN).await.unwrap(),
        0,
        "it fires once"
    );
    f.nothing().await;
}
fn parse(value: &serde_json::Value) -> i64 {
    boblm_server::schedule::parse_rfc3339(value.as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn a_repeating_schedule_skips_busy_runs_and_stops_after_its_last() {
    let mut f = fixture().await;
    f.app
        .schedule_prompt(
            "s",
            args(ID1, |a| {
                a.every_minutes = Some(1);
                a.max_runs = Some(2);
            }),
            NOW,
        )
        .await
        .unwrap();
    // The owner starts a turn, so the first run finds the chat busy.
    let handle = f.app.handle_for_tests("s").await;
    handle.send(WORK, "work").await.unwrap();
    f.next().await;
    f.events
        .send(Event::CommandLifecycle {
            command_uuid: WORK.into(),
            state: LifecycleState::Started,
        })
        .unwrap();
    for _ in 0..200 {
        if handle.status().await.unwrap().turn_active {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(f.app.run_due_schedules(NOW + MIN + 1000).await.unwrap(), 1);
    f.nothing().await;
    let s = &f.list("s").await[0];
    assert_eq!(
        (
            s["skipped"].as_i64(),
            s["runs"].as_i64(),
            s["state"].as_str()
        ),
        (Some(1), Some(0), Some("active"))
    );
    assert_eq!(parse(&s["nextRunAt"]), NOW + 2 * MIN);
    // Idle again: the next run is sent, then the last one finishes the schedule.
    f.events
        .send(Event::Result {
            subtype: Some("success".into()),
            is_error: Some(false),
        })
        .unwrap();
    f.idle().await;
    f.app.run_due_schedules(NOW + 2 * MIN + 1000).await.unwrap();
    assert!(
        f.next()
            .await
            .content
            .starts_with("Scheduled prompt (every 1 minute):")
    );
    let s = &f.list("s").await[0];
    assert_eq!(
        (s["runs"].as_i64(), s["state"].as_str()),
        (Some(1), Some("active"))
    );
    // Its own turn must finish before the final run goes out.
    f.events
        .send(Event::Result {
            subtype: Some("success".into()),
            is_error: Some(false),
        })
        .unwrap();
    f.events
        .send(Event::CommandLifecycle {
            command_uuid: boblm_server::schedule::message_id(ID1, NOW + 2 * MIN),
            state: LifecycleState::Completed,
        })
        .unwrap();
    f.idle().await;
    f.app.run_due_schedules(NOW + 3 * MIN + 1000).await.unwrap();
    f.next().await;
    let s = &f.list("s").await[0];
    assert_eq!(
        (s["runs"].as_i64(), s["state"].as_str()),
        (Some(2), Some("done"))
    );
}

#[tokio::test]
async fn downtime_is_not_made_up_for_repeating_schedules_and_limits_late_one_shots() {
    let mut f = fixture().await;
    f.app
        .schedule_prompt("s", args(ID1, |a| a.every_minutes = Some(5)), NOW)
        .await
        .unwrap();
    f.app
        .schedule_prompt("s", args(ID2, |a| a.in_minutes = Some(1)), NOW)
        .await
        .unwrap();
    let late = "30000000-0000-4000-8000-000000000003";
    f.app
        .schedule_prompt("s", args(late, |a| a.in_minutes = Some(2)), NOW)
        .await
        .unwrap();
    // The server was away for 90 minutes: the repeating schedule resumes at its next slot,
    // the one-shot 89 minutes late is dropped as missed, and nothing is sent.
    let back = NOW + 90 * MIN;
    assert_eq!(f.app.run_due_schedules(back).await.unwrap(), 3);
    f.nothing().await;
    let listed = f.list("s").await;
    let by_id = |id: &str| {
        listed
            .iter()
            .find(|s| s["scheduleId"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(by_id(ID1)["skipped"], 1);
    assert_eq!(
        parse(&by_id(ID1)["nextRunAt"]),
        NOW + 95 * MIN,
        "the first future slot"
    );
    assert_eq!(by_id(ID2)["state"], "missed");
    assert_eq!(by_id(late)["state"], "missed");
    // A one-shot only slightly late (within the grace period) is still sent.
    let near = "30000000-0000-4000-8000-000000000004";
    f.app
        .schedule_prompt("s", args(near, |a| a.in_minutes = Some(1)), back)
        .await
        .unwrap();
    assert_eq!(f.app.run_due_schedules(back + 31 * MIN).await.unwrap(), 2);
    assert!(f.next().await.content.starts_with("Scheduled prompt:"));
}

#[tokio::test]
async fn schedules_pause_while_archived_and_resume_on_reopen() {
    let mut f = fixture().await;
    f.app
        .schedule_prompt("s", args(ID1, |a| a.in_minutes = Some(1)), NOW)
        .await
        .unwrap();
    let handle = f.app.handle_for_tests("s").await;
    handle.archive(true).await.unwrap();
    assert_eq!(
        f.app.run_due_schedules(NOW + 2 * MIN).await.unwrap(),
        0,
        "archived chats are not scheduled"
    );
    f.nothing().await;
    assert_eq!(f.list("s").await[0]["state"], "active");
    handle.archive(false).await.unwrap();
    assert_eq!(f.app.run_due_schedules(NOW + 3 * MIN).await.unwrap(), 1);
    assert!(f.next().await.content.starts_with("Scheduled prompt:"));
}

#[tokio::test]
async fn creation_is_idempotent_limited_and_private_to_the_chat() {
    let f = fixture().await;
    let make = |id: &'static str| args(id, |a| a.in_minutes = Some(10));
    let first = f.app.schedule_prompt("s", make(ID1), NOW).await.unwrap();
    let again = f
        .app
        .schedule_prompt("s", make(ID1), NOW + 5000)
        .await
        .unwrap();
    assert_eq!(again["alreadyExisted"], true);
    assert_eq!(
        first["nextRunAt"], again["nextRunAt"],
        "a retry keeps the original timing"
    );
    // Uppercase spelling of the same ID is the same schedule.
    let upper = f.app.schedule_prompt("s", make(ID1), NOW).await.unwrap();
    assert_eq!(upper["scheduleId"], ID1);
    let different = f
        .app
        .schedule_prompt(
            "s",
            args(ID1, |a| {
                a.in_minutes = Some(10);
                a.prompt = "other".into();
            }),
            NOW,
        )
        .await
        .unwrap_err();
    assert!(
        different.contains("already belongs to a different schedule"),
        "{different}"
    );
    let foreign = f
        .app
        .schedule_prompt("t", make(ID1), NOW)
        .await
        .unwrap_err();
    assert!(
        foreign.contains("already belongs to a different schedule"),
        "{foreign}"
    );
    for n in 2..=20 {
        f.app
            .schedule_prompt(
                "s",
                make(&*Box::leak(
                    format!("30000000-0000-4000-8000-{n:012}").into_boxed_str(),
                )),
                NOW,
            )
            .await
            .unwrap();
    }
    let full = f
        .app
        .schedule_prompt("s", make("30000000-0000-4000-8000-000000000099"), NOW)
        .await
        .unwrap_err();
    assert!(
        full.contains("already has 20 active schedules") && full.contains("cancel one first"),
        "{full}"
    );
    // Another chat's schedules are neither listed nor cancellable.
    assert!(f.list("t").await.is_empty());
    let stranger = f.app.cancel_schedule("t", ID1).await.unwrap_err();
    assert!(
        stranger.contains("has no schedule") && stranger.contains("list_schedules"),
        "{stranger}"
    );
    assert_eq!(
        f.app.cancel_schedule("s", ID1).await.unwrap()["cancelled"],
        true
    );
    assert_eq!(f.list("s").await.len(), 19);
    let gone = f.app.cancel_schedule("s", ID1).await.unwrap_err();
    assert!(gone.contains("already be cancelled"), "{gone}");
    assert!(
        f.app
            .cancel_schedule("s", "nope")
            .await
            .unwrap_err()
            .contains("scheduleId must be a UUID")
    );
    let bad = f
        .app
        .schedule_prompt("s", args("nope", |a| a.in_minutes = Some(1)), NOW)
        .await
        .unwrap_err();
    assert!(bad.contains("scheduleId must be a fresh UUID"), "{bad}");
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_schedule_cancelled_after_the_due_list_was_read_is_not_sent() {
    let mut f = fixture().await;
    f.app
        .schedule_prompt(
            "s",
            args(ID1, |a| a.at = Some(format_rfc3339(NOW + MIN))),
            NOW,
        )
        .await
        .unwrap();
    // The scheduler's snapshot, taken before the cancel lands.
    let stale = f
        .store
        .reader()
        .unwrap()
        .due_schedules(NOW + MIN + 1000)
        .unwrap();
    assert_eq!(stale.len(), 1);
    f.app.cancel_schedule("s", ID1).await.unwrap();
    let acted = f
        .app
        .settle_due(stale[0].clone(), NOW + MIN + 1000)
        .await
        .unwrap();
    assert!(!acted, "a cancelled occurrence is not acted on");
    f.nothing().await;
}

#[tokio::test]
async fn retrying_a_one_shot_after_its_time_returns_the_schedule_instead_of_failing() {
    let f = fixture().await;
    let request = || args(ID1, |a| a.at = Some(format_rfc3339(NOW + MIN)));
    f.app.schedule_prompt("s", request(), NOW).await.unwrap();
    // The reply was lost; the agent asks again once the time has gone by.
    let again = f
        .app
        .schedule_prompt("s", request(), NOW + 5 * MIN)
        .await
        .unwrap();
    assert_eq!(again["alreadyExisted"], true);
    // A different request under the same ID, or from another chat, is still refused.
    assert!(
        f.app
            .schedule_prompt(
                "s",
                args(ID1, |a| a.at = Some(format_rfc3339(NOW + 9 * MIN))),
                NOW + 5 * MIN
            )
            .await
            .is_err()
    );
    // A brand-new schedule in the past is still refused by the clock check.
    assert!(
        f.app
            .schedule_prompt(
                "s",
                args(ID2, |a| a.at = Some(format_rfc3339(NOW + MIN))),
                NOW + 5 * MIN
            )
            .await
            .is_err()
    );
}
