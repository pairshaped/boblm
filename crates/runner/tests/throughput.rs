//! Ignored measurement: how many streamed events per second one session consumes.
//! Run with `cargo test -j4 -p boblm-runner --test throughput -- --ignored --nocapture`.
//! Before event batching a session managed about 150 events/s on a fsync-bound disk.
use async_trait::async_trait;
use boblm_protocol::{Event, Priority};
use boblm_runner::{Driver, DriverFactory, RunningSession};
use boblm_store::{Session, Store};
use std::{
    io,
    sync::Arc,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

struct Fake(mpsc::UnboundedReceiver<Event>);
#[async_trait]
impl Driver for Fake {
    async fn send(&mut self, _: &str, _: &str, _: Option<Priority>) -> io::Result<()> {
        Ok(())
    }
    async fn next_event(&mut self) -> Option<Event> {
        self.0.recv().await
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
        Ok(Box::new(self.0.lock().unwrap().take().unwrap()))
    }
}

async fn run(other_sessions: usize, messages: usize, events: usize) -> f64 {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("bench.sqlite")).unwrap();
    store.create_session("s", "/local/project", 1).unwrap();
    for i in 0..other_sessions {
        store.create_session(&format!("o{i}"), "/x", 1).unwrap();
    }
    for i in 0..messages {
        store
            .append(boblm_store::NewMessage {
                uuid: &format!("00000000-0000-4000-8000-{i:012}"),
                session: "s",
                role: "user",
                content: "hello",
                delivery_state: "started",
                created_at: 2,
            })
            .unwrap();
    }
    let (tx, rx) = mpsc::unbounded_channel();
    let runner = RunningSession::start(
        store.clone(),
        "s",
        Arc::new(Factory(Mutex::new(Some(Fake(rx))))),
    )
    .await
    .unwrap();
    let client = runner.handle();
    client
        .send("ffffffff-0000-4000-8000-000000000001", "go")
        .await
        .unwrap();
    let start = Instant::now();
    for n in 0..events {
        tx.send(Event::StreamEvent { event: serde_json::json!({"type":"content_block_delta","delta":{"text":format!("tok{n}")}}) }).unwrap();
    }
    tx.send(Event::Result {
        subtype: Some("success".into()),
        is_error: Some(false),
    })
    .unwrap();
    // Wait until every event is stored.
    loop {
        // Count agent_event rows directly through the cursor.
        let mut after = 0;
        let mut total = 0;
        loop {
            let page = store
                .reader()
                .unwrap()
                .events_since("s", after, 1000)
                .unwrap();
            if page.is_empty() {
                break;
            }
            total += page.iter().filter(|e| e.kind == "agent_event").count();
            after = page.last().unwrap().seq;
        }
        if total > events {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let rate = events as f64 / start.elapsed().as_secs_f64();
    runner.shutdown().await.unwrap();
    rate
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn measure() {
    for (others, messages) in [(0, 0), (50, 2000)] {
        let rate = run(others, messages, 2000).await;
        println!("other sessions {others}, history messages {messages}: {rate:.0} events/s");
    }
}
