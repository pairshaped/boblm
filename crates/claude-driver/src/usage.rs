//! Best-effort read-only controls. A missing answer never stops the child.
use super::*;
use std::{pin::Pin, task::Poll};
use tokio::io::AsyncWrite;

#[derive(Default)]
pub(super) struct Probes {
    pending: HashMap<String, (bool, tokio::time::Instant)>,
}
impl Probes {
    pub async fn poll(&mut self, stdin: &tokio::sync::Mutex<ChildStdin>, weekly: bool) {
        self.pending
            .retain(|_, (_, at)| at.elapsed() < boblm_tuning::USAGE_REQUEST_TIMEOUT);
        if self.pending.values().any(|(kind, _)| *kind == weekly) {
            return;
        }
        let request = if weekly {
            serde_json::json!({"subtype":"get_usage","skip_behaviors":true})
        } else {
            serde_json::json!({"subtype":"get_context_usage","detail":"summary"})
        };
        let id = format!("boblm-usage-{}", uuid::Uuid::new_v4());
        let mut line = serde_json::to_vec(
            &serde_json::json!({"type":"control_request","request_id":id,"request":request}),
        )
        .unwrap_or_default();
        line.push(b'\n');
        if line.len() > nix::libc::PIPE_BUF {
            return;
        }
        let Ok(mut input) = stdin.try_lock() else {
            return;
        };
        // These small frames fit PIPE_BUF. On a nonblocking pipe a write is atomic:
        // if it cannot fit, skip the sample instead of holding up an owner message.
        let written = std::future::poll_fn(|cx| {
            Poll::Ready(match Pin::new(&mut *input).poll_write(cx, &line) {
                Poll::Ready(Ok(count)) => count,
                _ => 0,
            })
        })
        .await;
        if written == line.len() {
            self.pending
                .insert(id, (weekly, tokio::time::Instant::now()));
        }
    }
    pub fn response(&mut self, response: &serde_json::Value) -> Option<Event> {
        let id = response["request_id"].as_str()?;
        let (weekly, at) = self.pending.remove(id)?;
        if at.elapsed() >= boblm_tuning::USAGE_REQUEST_TIMEOUT || response["subtype"] != "success" {
            return None;
        }
        let value = &response["response"];
        if weekly {
            let weekly = boblm_protocol::usage::claude_weekly(value);
            let five_hour = boblm_protocol::usage::claude_five_hour(value);
            (weekly.is_some() || five_hour.is_some()).then_some(Event::Usage {
                context: None,
                weekly,
                five_hour,
                credits: None,
            })
        } else {
            Some(Event::Usage {
                five_hour: None,
                credits: None,
                context: Some(boblm_protocol::usage::claude_context(value)?),
                weekly: None,
            })
        }
    }
}
