use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use boblm_protocol::{Event, LifecycleState, Priority};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Notify, mpsc};
use tower::ServiceExt;

struct Gate {
    entered: Arc<Notify>,
    release: Arc<Notify>,
    block_startup: bool,
}
struct Fake {
    tx: mpsc::UnboundedSender<Event>,
    rx: mpsc::UnboundedReceiver<Event>,
    send_gate: Option<(Arc<Notify>, Arc<Notify>)>,
}
#[async_trait]
impl DriverFactory for Gate {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        if self.block_startup {
            self.entered.notify_one();
            self.release.notified().await;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(Box::new(Fake {
            tx,
            rx,
            send_gate: if self.block_startup {
                None
            } else {
                Some((self.entered.clone(), self.release.clone()))
            },
        }))
    }
}
#[async_trait]
impl Driver for Fake {
    async fn send(&mut self, id: &str, _: &str, _: Option<Priority>) -> io::Result<()> {
        if let Some((entered, release)) = self.send_gate.take() {
            entered.notify_one();
            release.notified().await;
        }
        self.tx
            .send(Event::CommandLifecycle {
                command_uuid: id.into(),
                state: LifecycleState::Started,
            })
            .map_err(io::Error::other)?;
        Ok(())
    }
    async fn next_event(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
    async fn configure(&mut self, _: &str, _: Option<&str>) -> io::Result<()> {
        Ok(())
    }
    async fn abort(&mut self) -> io::Result<()> {
        Ok(())
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        self.rx.close();
        Ok(())
    }
}
fn command(name: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/commands/{name}"))
        .header("Authorization", "Bearer fake-latency-test-token")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}
#[tokio::test]
async fn saved_input_and_chat_list_are_visible_while_agent_startup_is_blocked() {
    responsiveness(true).await;
}
#[tokio::test]
async fn saved_input_and_chat_list_are_visible_while_agent_write_is_blocked() {
    responsiveness(false).await;
}
async fn responsiveness(block_startup: bool) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("latency.sqlite")).unwrap();
    store.create_session("s", "/fake", 1).unwrap();
    let gate = Arc::new(Gate {
        entered: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
        block_startup,
    });
    let app = App::open(
        store.clone(),
        gate.clone(),
        "fake-latency-test-token".into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    let id = "00000000-0000-4000-8000-000000000099";
    let start = Instant::now();
    let send_router = router.clone();
    let mut send = tokio::spawn(async move {
        send_router.oneshot(command("sendMessage",serde_json::json!({"sessionId":"s","messageId":id,"content":"Show this queued input immediately"}))).await.unwrap()
    });
    tokio::time::timeout(Duration::from_secs(10), gate.entered.notified())
        .await
        .unwrap();
    let saved_after = start.elapsed();
    let saved = store.reader().unwrap().message(id).unwrap().unwrap();
    assert_eq!(
        saved.delivery_state,
        if block_startup {
            "queued"
        } else {
            "dispatching"
        }
    );
    let receipt_start = Instant::now();
    let response = tokio::time::timeout(Duration::from_millis(300), &mut send).await;
    let receipt_ms = receipt_start.elapsed().as_millis();
    let send_ready = response.is_ok();
    let event_start = Instant::now();
    let events_router = router.clone();
    let visible = tokio::time::timeout(Duration::from_millis(300), async move {
        let response = events_router
            .oneshot(
                Request::builder()
                    .uri("/sessions/s/events")
                    .header("Authorization", "Bearer fake-latency-test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut body = response.into_body();
        let mut received = String::new();
        while let Some(frame) = body.frame().await {
            if let Ok(bytes) = frame.unwrap().into_data() {
                received.push_str(&String::from_utf8_lossy(&bytes));
            }
            if received.contains("Show this queued input immediately") {
                return true;
            }
        }
        false
    })
    .await;
    let event_ms = event_start.elapsed().as_millis();
    let list_start = Instant::now();
    let list = tokio::time::timeout(
        Duration::from_millis(300),
        router
            .clone()
            .oneshot(command("listSessions", serde_json::json!({}))),
    )
    .await;
    let list_ms = list_start.elapsed().as_millis();
    let list_ready = list.is_ok();
    // Always release startup before asserting, including on the original failure path.
    gate.release.notify_one();
    let reply = match response {
        Ok(value) => value.unwrap(),
        Err(_) => send.await.unwrap(),
    };
    assert_eq!(reply.status(), StatusCode::OK);
    let until = Instant::now() + Duration::from_secs(3);
    while store
        .reader()
        .unwrap()
        .message(id)
        .unwrap()
        .unwrap()
        .delivery_state
        != "started"
    {
        assert!(
            Instant::now() < until,
            "the fake child's lifecycle acknowledgment was not consumed"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    app.shutdown().await.unwrap();
    eprintln!(
        "startup_gate={} saved_after_ms={} receipt_ms={} event_ms={} list_ms={} send_ready={} event_visible={} list_ready={}",
        block_startup,
        saved_after.as_millis(),
        receipt_ms,
        event_ms,
        list_ms,
        send_ready,
        visible.as_ref().is_ok_and(|v| *v),
        list_ready
    );
    assert!(
        send_ready,
        "the saved-input receipt waited for agent startup"
    );
    assert!(
        visible.is_ok_and(|value| value),
        "saved input could not reach the browser while startup was blocked"
    );
    assert!(list_ready, "the chat list waited for agent startup");
}
