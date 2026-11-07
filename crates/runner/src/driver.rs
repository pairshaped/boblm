use async_trait::async_trait;
use boblm_claude_driver::ClaudeSession;
use boblm_protocol::{Event, Priority};
use boblm_store::Session;
use std::{io, path::Path, sync::Arc};

#[async_trait]
pub trait Driver: Send + Sync {
    async fn send(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
    ) -> io::Result<()>;
    async fn send_with_origin(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
        _human: bool,
    ) -> io::Result<()> {
        self.send(uuid, content, priority).await
    }
    async fn next_event(&mut self) -> Option<Event>;
    async fn configure(&mut self, model: &str, effort: Option<&str>) -> io::Result<()>;
    /// True when a steer waits for the next tool boundary instead of being consumed promptly.
    fn steers_wait_for_tool_boundary(&self) -> bool {
        false
    }
    /// True when the provider starts its own turn when a background command finishes.
    fn wakes_itself_on_background_job_completion(&self) -> bool {
        false
    }
    /// The driver skips its periodic usage poll while this returns true.
    fn set_usage_poll_gate(&mut self, _gate: Arc<dyn Fn() -> bool + Send + Sync>) {}
    async fn abort(&mut self) -> io::Result<()>;
    async fn shutdown(&mut self) -> io::Result<()>;
}

#[async_trait]
pub trait DriverFactory: Send + Sync {
    fn external_runners(&self) -> bool {
        false
    }
    /// Production can attach to a persistent runner instead of owning a local actor.
    async fn connect_runner(
        &self,
        _store: &boblm_store::Store,
        _session: &str,
    ) -> crate::Result<Option<crate::RunningSession>> {
        Ok(None)
    }
    async fn spawn(&self, session: &Session) -> io::Result<Box<dyn Driver>>;
    /// Read the provider account this child will run on, when its local files expose one.
    fn account_id(&self, _session: &Session) -> Option<String> {
        None
    }
}

pub struct ClaudeFactory;
#[async_trait]
impl DriverFactory for ClaudeFactory {
    fn account_id(&self, _session: &Session) -> Option<String> {
        boblm_claude_driver::account::account_id()
    }
    async fn spawn(&self, session: &Session) -> io::Result<Box<dyn Driver>> {
        Ok(Box::new(ClaudeDriver(Some(
            ClaudeSession::resume(
                Path::new(&session.working_directory),
                session.agent_session_id.as_deref(),
            )
            .await?,
        ))))
    }
}

/// Wrap an injected ClaudeSession to exercise Runner against a fake executable.
pub struct ClaudeDriver(pub Option<ClaudeSession>);
#[async_trait]
impl Driver for ClaudeDriver {
    fn wakes_itself_on_background_job_completion(&self) -> bool {
        true
    }
    async fn send(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
    ) -> io::Result<()> {
        self.0
            .as_ref()
            .ok_or_else(|| {
                io::Error::other("Claude is not running: its process was already stopped.")
            })?
            .send_message(uuid, content, priority)
            .await
    }
    async fn send_with_origin(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
        human: bool,
    ) -> io::Result<()> {
        self.0
            .as_ref()
            .ok_or_else(|| {
                io::Error::other("Claude is not running: its process was already stopped.")
            })?
            .send_message_with_origin(uuid, content, priority, human)
            .await
    }
    async fn next_event(&mut self) -> Option<Event> {
        match &mut self.0 {
            Some(session) => session.events.recv().await,
            None => None,
        }
    }
    async fn configure(&mut self, model: &str, effort: Option<&str>) -> io::Result<()> {
        let session = self.0.as_ref().ok_or_else(|| {
            io::Error::other("Claude is not running: its process was already stopped.")
        })?;
        session.enable_usage_reporting();
        session.set_model(Some(model)).await?;
        session.set_effort(effort).await
    }
    fn set_usage_poll_gate(&mut self, gate: Arc<dyn Fn() -> bool + Send + Sync>) {
        if let Some(session) = &self.0 {
            session.set_usage_poll_gate(gate);
        }
    }
    async fn abort(&mut self) -> io::Result<()> {
        self.0
            .as_ref()
            .ok_or_else(|| {
                io::Error::other("Claude is not running: its process was already stopped.")
            })?
            .request_interrupt()
            .await
            .map(|_| ())
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        if let Some(session) = self.0.take() {
            // Dropping the session on timeout kills its group and aborts the reader.
            tokio::time::timeout(boblm_tuning::DRIVER_SHUTDOWN_TIMEOUT, session.stop())
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "Claude shutdown timed out")
                })??;
        }
        Ok(())
    }
}

/// Wrap a Codex session so the runner drives it exactly as it drives Claude.
pub struct CodexDriver(pub Option<boblm_codex_driver::CodexSession>);
#[async_trait]
impl Driver for CodexDriver {
    fn steers_wait_for_tool_boundary(&self) -> bool {
        true
    }
    async fn send(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
    ) -> io::Result<()> {
        self.0
            .as_ref()
            .ok_or_else(|| {
                io::Error::other("Codex is not running: its process was already stopped.")
            })?
            .send_message(uuid, content, priority)
            .await
    }
    async fn next_event(&mut self) -> Option<Event> {
        match &mut self.0 {
            Some(session) => session.events.recv().await,
            None => None,
        }
    }
    async fn configure(&mut self, model: &str, effort: Option<&str>) -> io::Result<()> {
        if let Some(session) = &mut self.0 {
            session.enable_usage_reporting();
            session.enable_background_reporting();
        }
        self.0
            .as_ref()
            .ok_or_else(|| {
                io::Error::other("Codex is not running: its process was already stopped.")
            })?
            .configure(model, effort);
        Ok(())
    }
    fn set_usage_poll_gate(&mut self, gate: Arc<dyn Fn() -> bool + Send + Sync>) {
        if let Some(session) = &self.0 {
            session.set_usage_poll_gate(gate);
        }
    }
    async fn abort(&mut self) -> io::Result<()> {
        self.0
            .as_ref()
            .ok_or_else(|| {
                io::Error::other("Codex is not running: its process was already stopped.")
            })?
            .request_interrupt()
            .await
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        if let Some(session) = self.0.take() {
            tokio::time::timeout(boblm_tuning::DRIVER_SHUTDOWN_TIMEOUT, session.stop())
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "Codex shutdown timed out")
                })??;
        }
        Ok(())
    }
}

/// Wrap a Pi session so the runner drives it exactly as it drives Claude.
pub struct PiDriver(pub Option<boblm_pi_driver::PiSession>);
#[async_trait]
impl Driver for PiDriver {
    fn steers_wait_for_tool_boundary(&self) -> bool {
        true
    }
    async fn send(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
    ) -> io::Result<()> {
        self.0
            .as_ref()
            .ok_or_else(|| io::Error::other("Pi is not running: its process was already stopped."))?
            .send_message(uuid, content, priority)
            .await
    }
    async fn next_event(&mut self) -> Option<Event> {
        match &mut self.0 {
            Some(session) => session.events.recv().await,
            None => None,
        }
    }
    async fn configure(&mut self, model: &str, effort: Option<&str>) -> io::Result<()> {
        if let Some(session) = &self.0 {
            session.enable_usage_reporting();
        }
        self.0
            .as_ref()
            .ok_or_else(|| io::Error::other("Pi is not running: its process was already stopped."))?
            .configure(model, effort)
            .await
    }
    async fn abort(&mut self) -> io::Result<()> {
        self.0
            .as_ref()
            .ok_or_else(|| io::Error::other("Pi is not running: its process was already stopped."))?
            .request_interrupt()
            .await
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        if let Some(session) = self.0.take() {
            tokio::time::timeout(boblm_tuning::DRIVER_SHUTDOWN_TIMEOUT, session.stop())
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Pi did not finish shutting down within the deadline. Its process group was stopped; reopen the chat."))??;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pi_and_codex_wait_for_a_tool_boundary_before_consuming_a_steer() {
        assert!(PiDriver(None).steers_wait_for_tool_boundary());
        assert!(CodexDriver(None).steers_wait_for_tool_boundary());
        assert!(!ClaudeDriver(None).steers_wait_for_tool_boundary());
    }

    #[test]
    fn only_claude_wakes_itself_on_background_job_completion() {
        assert!(ClaudeDriver(None).wakes_itself_on_background_job_completion());
        assert!(!CodexDriver(None).wakes_itself_on_background_job_completion());
        assert!(!PiDriver(None).wakes_itself_on_background_job_completion());
    }
}
