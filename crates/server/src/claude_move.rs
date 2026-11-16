//! The owner's manual move of an idle Claude plan chat to another plan account (ADR 0047).
//!
//! The server performs the move through paths every runner already supports, so a runner
//! started from an older binary needs no new code: archive stops the idle child, the
//! transcript is copied to the destination home, one store transaction switches the
//! account, and reopen leaves the chat to start its next child with `--resume` on the
//! destination account. Nothing here starts a fresh conversation.
use crate::app::{ApiError, ApiResult, App, body};
use crate::claude_account::{self, plan_label};
use axum::{Json, extract::State, http::StatusCode};
use boblm_protocol::client::{ClaudeMoveStatus, MoveClaudeAccount, MoveClaudeAccountReply, shown};
use boblm_runner::SessionHandle;
use boblm_tuning::{CLAUDE_ACCOUNT_KEY_PLAN_A, CLAUDE_PLAN_ACCOUNTS};
use std::sync::Arc;

pub(crate) async fn move_account(
    State(app): State<Arc<App>>,
    bytes: axum::body::Bytes,
) -> ApiResult<MoveClaudeAccountReply> {
    let request: MoveClaudeAccount = body(&bytes)?;
    app.move_claude_account(&request.session_id, &request.account_key)
        .await
        .map(Json)
}

/// Releases the chat's move slot however the move ends.
struct MoveSlot<'a>(&'a App, String);
impl Drop for MoveSlot<'_> {
    fn drop(&mut self) {
        self.0
            .claude_moves
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.1);
    }
}

fn refused(message: String) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "move_refused", message)
}
fn busy(message: String) -> ApiError {
    ApiError(StatusCode::CONFLICT, "not_idle", message)
}
fn failed(message: String) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, "move_failed", message)
}

/// Leave the chat open again after a stopped or abandoned move. A chat that an owner
/// message already reopened is left alone: that message decides what runs next.
async fn reopen_if_archived(handle: &SessionHandle) -> Result<(), String> {
    let status = handle.status().await.map_err(|error| error.to_string())?;
    if status.archived {
        handle
            .archive(false)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

impl App {
    pub(crate) async fn move_claude_account(
        &self,
        id: &str,
        target: &str,
    ) -> Result<MoveClaudeAccountReply, ApiError> {
        let handle = self.handle(id).await?;
        let store = self.store.clone();
        let lookup = id.to_owned();
        let (session, handoffs, subagents, outstanding, activity) =
            tokio::task::spawn_blocking(move || {
                let reader = store.reader()?;
                let session = reader.sessions()?.into_iter().find(|s| s.id == lookup);
                Ok::<_, boblm_store::Error>((
                    session,
                    reader.claude_account_handoffs(&lookup)?,
                    reader.subagent_summary(&lookup)?,
                    reader.outstanding_messages(&lookup)?,
                    reader.runner_restart_activity(&lookup)?,
                ))
            })
            .await
            .map_err(ApiError::internal)??;
        let session = session.ok_or_else(|| {
            refused(format!(
                "There is no chat with ID {} to move. Reload the chat list.",
                shown(id)
            ))
        })?;
        let name = if session.name.is_empty() {
            "This chat".to_owned()
        } else {
            format!("The chat \"{}\"", shown(&session.name))
        };
        if session.agent != "claude" {
            let agent = match session.agent.as_str() {
                "codex" => "Codex",
                "pi" => "Pi",
                other => other,
            };
            return Err(refused(format!(
                "{name} runs {agent}, and only Claude plan chats move between Claude accounts. Nothing was changed."
            )));
        }
        if session.parent_session_id.is_some() || session.root_session_id.is_some() {
            return Err(refused(format!(
                "{name} is a subagent. Subagents never move accounts; move the chat that owns it after its subagents finish. Nothing was changed."
            )));
        }
        let source = session
            .account_key
            .clone()
            .unwrap_or_else(|| CLAUDE_ACCOUNT_KEY_PLAN_A.into());
        if claude_account::is_api(&source) {
            return Err(refused(format!(
                "{name} runs on a Claude API account, and API chats never move. Nothing was changed."
            )));
        }
        if !CLAUDE_PLAN_ACCOUNTS.contains(&target) {
            return Err(refused(format!(
                "{} is not a Claude plan account. Use claude-plan-a or claude-plan-b; API accounts are never a destination. Nothing was changed.",
                shown(target)
            )));
        }
        let enabled = self.claude_accounts();
        if !enabled.plan.contains(&target) {
            return Err(refused(format!(
                "{} is not enabled on this server. Log in to its Claude home, add it to CLAUDE_PLAN_ACCOUNTS in crates/tuning, and rebuild. Nothing was changed.",
                plan_label(target)
            )));
        }
        let (from, to) = (plan_label(&source), plan_label(target));
        if source == target {
            if handoffs.is_empty() {
                return Err(refused(format!(
                    "{name} already runs on {to}, so there is nothing to move. Nothing was changed."
                )));
            }
            // A retry of a move that already finished: the account, the copy and the
            // no-return exclusion are all in place.
            let previous = handoffs
                .iter()
                .map(|(key, _)| key.as_str())
                .find(|key| *key != target)
                .map_or_else(|| from.clone(), plan_label);
            return Ok(MoveClaudeAccountReply {
                session_id: id.to_owned(),
                from: previous,
                to,
                status: ClaudeMoveStatus::AlreadyMoved,
            });
        }
        let _slot = {
            let mut moves = self
                .claude_moves
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !moves.insert(id.to_owned()) {
                return Err(busy(format!(
                    "{name} is already being moved to another Claude account. Wait for that move to finish. Nothing was changed."
                )));
            }
            MoveSlot(self, id.to_owned())
        };

        // Idle means the same thing as for a runner restart: nothing running or waiting,
        // and a turn result old enough that no late acknowledgment can still arrive.
        let status = handle.status().await?;
        let mut reasons = Vec::new();
        if status.archived {
            reasons.push("it is archived".to_owned());
        }
        if status.turn_active || status.stopping {
            reasons.push("a turn is running or stopping".to_owned());
        }
        let waiting = status.queued + status.dispatching + status.uncertain;
        let stored = outstanding
            .iter()
            .filter(|m| {
                matches!(
                    m.delivery_state.as_str(),
                    "queued" | "dispatching" | "uncertain"
                )
            })
            .count();
        if waiting > 0 || stored > 0 {
            reasons.push(format!(
                "{} messages are queued or waiting for delivery",
                waiting.max(stored)
            ));
        }
        // A held queue with nothing queued is harmless and reopening leaves it as it was.
        // A stop is the owner's own decision, which reopening would clear.
        if status.stopped {
            reasons.push(
                "the owner stopped it and has not sent a message since, so send a message or press Resume first"
                    .to_owned(),
            );
        }
        match &status.jobs {
            Some(jobs) if jobs.available && jobs.tasks.is_empty() => {}
            Some(jobs) if jobs.available => {
                reasons.push(format!("{} background jobs are running", jobs.tasks.len()))
            }
            None if !status.running => {}
            _ => reasons.push("its background jobs cannot be read right now".to_owned()),
        }
        if subagents.open > 0 {
            reasons.push(format!(
                "{} subagents are still open, so let them finish or archive them",
                subagents.open
            ));
        }
        if !reasons.is_empty() {
            return Err(busy(format!(
                "{name} is not idle: {}. It stays on {from}. Try again when it is idle.",
                reasons.join("; ")
            )));
        }
        let quiet = match &activity {
            None => session.agent_session_id.is_none(),
            event => crate::persistent::result_is_quiet(event.as_ref()),
        };
        if !quiet {
            // Nothing is running, but a late acknowledgment could still arrive. This one
            // clears by itself, so callers can retry it without waiting for work to end.
            return Err(ApiError(
                StatusCode::CONFLICT,
                "settling",
                format!(
                    "{name} finished its last turn less than {} seconds ago, or never completed one, so it is not settled yet. It stays on {from}. Try again when it is idle.",
                    boblm_tuning::RUNNER_QUIET_PERIOD.as_secs()
                ),
            ));
        }

        let destination = boblm_store::Session {
            account_key: Some(target.to_owned()),
            ..session.clone()
        };
        let Some(account_id) = self.factory.account_id(&destination) else {
            return Err(refused(format!(
                "{to} has no signed-in Claude account in its config folder, so a chat cannot resume there. Log in under that account's home once (see docs/running.md), then try again. Nothing was changed."
            )));
        };
        let store = self.store.clone();
        let (source_key, target_key) = (source.clone(), target.to_owned());
        let plan = enabled.plan;
        let change = tokio::task::spawn_blocking(move || {
            claude_account::manual_handoff(
                &store,
                &source_key,
                &target_key,
                Some(account_id),
                plan,
                chrono::Utc::now().timestamp(),
            )
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;

        // Stop the idle child through the owner's own archive path.
        if let Err(error) = handle.archive(true).await {
            let _ = reopen_if_archived(&handle).await;
            return Err(failed(format!(
                "{name} could not be stopped for the move: {error}. It stays on {from}; check that its runner is running and try again."
            )));
        }
        let stopped = handle.status().await?;
        if stopped.running
            || stopped.turn_active
            || stopped.stopping
            || stopped.queued + stopped.dispatching + stopped.uncertain > 0
        {
            let _ = reopen_if_archived(&handle).await;
            return Err(busy(format!(
                "{name} started work while it was being stopped, so it was not moved. It stays on {from}. Try again when it is idle."
            )));
        }

        // A failed copy keeps the source account. It never falls back to a fresh conversation.
        if let Err(error) = self.factory.copy_claude_transcript(&session, target).await {
            let reopened = reopen_if_archived(&handle).await;
            return Err(failed(format!(
                "The conversation could not be copied to {to}: {error}. {name} stays on {from} and was not moved.{}",
                match reopened {
                    Ok(()) => String::new(),
                    Err(cause) =>
                        format!(" It could not be reopened ({cause}), so press Reopen on it."),
                }
            )));
        }
        let store = self.store.clone();
        let lookup = id.to_owned();
        let moved = tokio::task::spawn_blocking(move || {
            store.move_claude_account(&lookup, &change, chrono::Utc::now().timestamp_millis())
        })
        .await
        .map_err(ApiError::internal)?;
        match moved {
            Ok(true) => {}
            Ok(false) => {
                let _ = reopen_if_archived(&handle).await;
                return Err(busy(format!(
                    "{name} received input or changed while it was being moved, so it was not moved. It stays on {from}. Try again when it is idle."
                )));
            }
            Err(error) => {
                let _ = reopen_if_archived(&handle).await;
                return Err(failed(format!(
                    "The account switch could not be saved: {error}. {name} stays on {from}; try again."
                )));
            }
        }
        if let Err(cause) = reopen_if_archived(&handle).await {
            return Err(failed(format!(
                "{name} now runs on {to}, but it could not be reopened: {cause}. Press Reopen on it; its next message resumes the same conversation on {to}."
            )));
        }
        Ok(MoveClaudeAccountReply {
            session_id: id.to_owned(),
            from,
            to,
            status: ClaudeMoveStatus::Moved,
        })
    }
}
