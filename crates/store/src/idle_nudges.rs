//! One durable opportunity per root result, including results written by loaded runners.
#[path = "idle_nudges.generated.rs"]
mod sql;

use crate::{Reader, Result, Store};
pub use sql::{DueRow as IdleNudge, InputsRow as IdleNudgeInput};

impl Reader {
    /// Shared job-age rule for idle nudges and subagent result reports.
    pub fn job_quiet_check(
        &self,
        session: &str,
        running: bool,
        jobs: Option<&boblm_protocol::jobs::JobSnapshot>,
        now: i64,
    ) -> Result<QuietCheck> {
        jobs_quiet(&self.conn, session, running, jobs, now)
    }

    /// Only recent listed jobs count as future wakes for idle nudges (ADR 0046).
    pub fn idle_nudge_jobs_quiet(
        &self,
        session: &str,
        running: bool,
        jobs: Option<&boblm_protocol::jobs::JobSnapshot>,
        now: i64,
    ) -> Result<bool> {
        Ok(matches!(
            jobs_quiet(&self.conn, session, running, jobs, now)?,
            QuietCheck::Quiet
        ))
    }

    pub fn due_idle_nudges(&self, now: i64) -> Result<Vec<IdleNudge>> {
        let cutoff = now.saturating_sub(boblm_tuning::IDLE_NUDGE_DELAY.as_millis() as i64);
        Ok(sql::due(&self.conn, sql::DueParams { cutoff, now })?)
    }

    pub fn idle_nudge_input(
        &self,
        session: &str,
        result_seq: i64,
    ) -> Result<Option<IdleNudgeInput>> {
        Ok(sql::inputs(
            &self.conn,
            sql::InputsParams {
                session,
                result_seq,
            },
        )?
        .pop())
    }

    pub fn idle_nudge_report_sources(&self, child: &str, parent: &str) -> Result<Vec<i64>> {
        Ok(sql::report_sources(
            &self.conn,
            sql::ReportSourcesParams { child, parent },
        )?)
    }
}

impl Store {
    /// Accept and settle together, so a crash cannot forget a drop or duplicate a message.
    /// The ordinary automated send subsequently refreshes the actor's queue using this UUID.
    pub fn prepare_idle_nudge(
        &self,
        session: &str,
        result_seq: i64,
        message_id: &str,
        quiet: bool,
        now: i64,
    ) -> Result<bool> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        let Some(row) = sql::lookup(
            &tx,
            sql::LookupParams {
                session,
                result_seq,
            },
        )?
        .pop() else {
            return Ok(false);
        };
        if row.state != "pending" {
            return Ok(row.state == "queued");
        }
        let eligible = quiet
            && sql::eligible(
                &tx,
                sql::EligibleParams {
                    session,
                    result_seq,
                },
            )?
            .pop()
            .unwrap_or(0)
                != 0;
        // Jobs can defer only after every other guard passes, including guards
        // on children visited after one with a recent job.
        let check = if eligible {
            snapshots_quiet(&tx, session, result_seq, now)?
        } else {
            QuietCheck::Blocked
        };
        if let QuietCheck::RecentJobsUntil(recheck_at) = check {
            sql::defer(
                &tx,
                sql::DeferParams {
                    session,
                    result_seq,
                    recheck_at: Some(recheck_at),
                },
            )?;
            tx.commit()?;
            return Ok(false);
        }
        let eligible = matches!(check, QuietCheck::Quiet);
        if eligible {
            crate::messages::sql::append(
                &tx,
                crate::NewMessage {
                    uuid: message_id,
                    session,
                    role: "user",
                    content: boblm_tuning::IDLE_NUDGE_TEXT,
                    delivery_state: "queued",
                    created_at: now,
                },
            )?;
            crate::events::append(
                &tx,
                session,
                "message_appended",
                serde_json::json!({
                    "uuid": message_id, "role": "user", "content": boblm_tuning::IDLE_NUDGE_TEXT,
                    "delivery_state": "queued"
                }),
                now,
            )?;
        }
        sql::settle(
            &tx,
            sql::SettleParams {
                session,
                result_seq,
                state: if eligible { "queued" } else { "dropped" },
                message_id: Some(message_id),
            },
        )?;
        tx.commit()?;
        Ok(eligible)
    }

    pub fn finish_idle_nudge(
        &self,
        session: &str,
        result_seq: i64,
        message_id: &str,
    ) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        sql::settle(
            &mut *conn,
            sql::SettleParams {
                session,
                result_seq,
                state: "sent",
                message_id: Some(message_id),
            },
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuietCheck {
    Quiet,
    RecentJobsUntil(i64),
    Blocked,
}

/// Recheck persisted actor state under the same write lock as acceptance. Any event
/// the actor has not yet reflected in its summary makes the snapshot inconclusive.
fn snapshots_quiet(
    conn: &rusqlite::Connection,
    session: &str,
    result_seq: i64,
    now: i64,
) -> Result<QuietCheck> {
    let mut recheck_at = None;
    for row in sql::guard_statuses(
        conn,
        sql::GuardStatusesParams {
            session,
            result_seq,
        },
    )? {
        let Some(raw) = row.payload else {
            return Ok(QuietCheck::Blocked);
        };
        let status: boblm_protocol::client::SessionStatus =
            serde_json::from_str(&raw).map_err(std::io::Error::other)?;
        if !native_subagents_quiet(conn, &row.id)?
            || status.turn_active
            || status.stopping
            || (row.id == session && status.queue_held)
            || status.queued != 0
            || status.dispatching != 0
            || (row.id == session && status.uncertain != 0)
            || (row.id == session && status.rejected != 0)
            || row.unpublished != Some(0)
        {
            return Ok(QuietCheck::Blocked);
        }
        match jobs_quiet(conn, &row.id, status.running, status.jobs.as_ref(), now)? {
            QuietCheck::Blocked => return Ok(QuietCheck::Blocked),
            QuietCheck::RecentJobsUntil(until) => {
                recheck_at = Some(recheck_at.unwrap_or(until).max(until));
            }
            QuietCheck::Quiet => {}
        }
    }
    Ok(recheck_at.map_or(QuietCheck::Quiet, QuietCheck::RecentJobsUntil))
}

/// Recent jobs provide a deadline; missing inventories or ages cannot justify deferral.
fn jobs_quiet(
    conn: &rusqlite::Connection,
    session: &str,
    running: bool,
    jobs: Option<&boblm_protocol::jobs::JobSnapshot>,
    now: i64,
) -> Result<QuietCheck> {
    if !running {
        return Ok(QuietCheck::Quiet);
    }
    let Some(jobs) = jobs.filter(|jobs| jobs.available) else {
        return Ok(QuietCheck::Blocked);
    };
    let cutoff = now.saturating_sub(boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64);
    let mut recheck_at = None;
    for job in &jobs.tasks {
        let first_seen = sql::job_first_seen(
            conn,
            sql::JobFirstSeenParams {
                session,
                job_id: &job.id,
            },
        )?
        .pop();
        // Missing evidence cannot establish that a job is long-lived.
        let Some(first_seen) = first_seen else {
            return Ok(QuietCheck::Blocked);
        };
        if first_seen >= cutoff {
            // The exact window boundary still blocks, so check one millisecond later.
            let until = first_seen
                .saturating_add(boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64)
                .saturating_add(1);
            recheck_at = Some(recheck_at.unwrap_or(until).max(until));
        }
    }
    Ok(recheck_at.map_or(QuietCheck::Quiet, QuietCheck::RecentJobsUntil))
}

/// Retained native lifecycle events also cover chats whose runners predate role subagents.
fn native_subagents_quiet(conn: &rusqlite::Connection, session: &str) -> Result<bool> {
    let mut claude = std::collections::HashSet::new();
    let mut codex = std::collections::HashSet::new();
    for raw in sql::native_subagent_events(conn, sql::NativeSubagentEventsParams { session })? {
        let event: serde_json::Value = serde_json::from_str(&raw).map_err(std::io::Error::other)?;
        for block in event["message"]["content"].as_array().into_iter().flatten() {
            if block["type"] == "tool_result" {
                if let Some(id) = block["tool_use_id"].as_str() {
                    claude.remove(id);
                }
            } else if block["type"] == "tool_use" {
                match block["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "agent" | "task" => {
                        let Some(id) = block["id"].as_str() else {
                            return Ok(false);
                        };
                        claude.insert(id.to_owned());
                    }
                    "subagentactivity" => {
                        let input = &block["input"];
                        if input["agentPath"] == "/root" {
                            continue;
                        }
                        let Some(id) = input["agentThreadId"].as_str() else {
                            return Ok(false);
                        };
                        match input["kind"].as_str() {
                            Some("started") => {
                                codex.insert(id.to_owned());
                            }
                            Some("completed") => {
                                codex.remove(id);
                            }
                            _ => return Ok(false),
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(claude.is_empty() && codex.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use boblm_protocol::jobs::{BackgroundJob, JobSnapshot};

    #[test]
    fn idle_nudge_migration_gives_unknown_jobs_a_full_durable_window() {
        for summary in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let database = temp.path().join("old.sqlite");
            let migrations = temp.path().join("migrations");
            std::fs::create_dir(&migrations).unwrap();
            for (name, sql) in crate::migrations::MIGRATIONS {
                if *name < "050" {
                    std::fs::write(migrations.join(name), sql).unwrap();
                }
            }
            {
                let conn = rusqlite::Connection::open(&database).unwrap();
                marmot::migrations::migrate_connection(&conn, &migrations, "schema_versions")
                    .unwrap();
            }
            let snapshot = JobSnapshot {
                available: true,
                tasks: vec![BackgroundJob {
                    id: "preview".into(),
                    command: "preview".into(),
                    agent_key: None,
                }],
            };
            {
                let conn = rusqlite::Connection::open(&database).unwrap();
                conn.execute("INSERT INTO projects(id, path, name, created_at) VALUES ('project', '/tmp', 'Test', 1)", []).unwrap();
                conn.execute("INSERT INTO sessions(id, working_directory, project_id, created_at) VALUES ('root', '/tmp', 'project', 1)", []).unwrap();
                // An older inventory must not backfill disappeared jobs.
                conn.execute("INSERT INTO events(session, seq, kind, payload, created_at) VALUES ('root', 1, 'agent_event', ?1, 1)",
                    [serde_json::json!({"type":"background_jobs","snapshot":{"available":true,"tasks":[{"id":"gone"}]}}).to_string()]).unwrap();
                let (kind, payload) = if summary {
                    ("state_summary", serde_json::json!({"jobs":snapshot}))
                } else {
                    (
                        "agent_event",
                        serde_json::json!({"type":"background_jobs","snapshot":snapshot}),
                    )
                };
                conn.execute("INSERT INTO events(session, seq, kind, payload, created_at) VALUES ('root', 2, ?1, ?2, 2)",
                    rusqlite::params![kind, payload.to_string()]).unwrap();
            }
            let before = crate::events::now_ms();
            let store = Store::open(&database).unwrap();
            let after = crate::events::now_ms();
            let reader = store.reader().unwrap();
            let seen = sql::job_first_seen(
                &reader.conn,
                sql::JobFirstSeenParams {
                    session: "root",
                    job_id: "preview",
                },
            )
            .unwrap()[0];
            assert!((before..=after).contains(&seen));
            assert!(
                sql::job_first_seen(
                    &reader.conn,
                    sql::JobFirstSeenParams {
                        session: "root",
                        job_id: "gone"
                    }
                )
                .unwrap()
                .is_empty()
            );
            drop(store);
            let restarted = Store::open(&database).unwrap();
            let window = boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64;
            for now in [seen, seen + window - 1, seen + window] {
                assert!(
                    !restarted
                        .reader()
                        .unwrap()
                        .idle_nudge_jobs_quiet("root", true, Some(&snapshot), now)
                        .unwrap()
                );
            }
            let old = seen + window + 1;
            assert!(
                restarted
                    .reader()
                    .unwrap()
                    .idle_nudge_jobs_quiet("root", true, Some(&snapshot), old)
                    .unwrap()
            );
            // Removing and reusing an ID does not renew its observation.
            for tasks in [
                serde_json::json!([]),
                serde_json::to_value(&snapshot.tasks).unwrap(),
            ] {
                restarted.record_event("root", "agent_event", serde_json::json!({"type":"background_jobs","snapshot":{"available":true,"tasks":tasks}}), old).unwrap();
            }
            assert!(
                restarted
                    .reader()
                    .unwrap()
                    .idle_nudge_jobs_quiet("root", true, Some(&snapshot), old)
                    .unwrap()
            );
            let unavailable = JobSnapshot {
                available: false,
                ..snapshot.clone()
            };
            assert!(
                !restarted
                    .reader()
                    .unwrap()
                    .idle_nudge_jobs_quiet("root", true, Some(&unavailable), old)
                    .unwrap()
            );
            assert!(
                !restarted
                    .reader()
                    .unwrap()
                    .idle_nudge_jobs_quiet("root", true, None, old)
                    .unwrap()
            );
            assert!(
                !restarted
                    .reader()
                    .unwrap()
                    .idle_nudge_jobs_quiet("unknown", true, Some(&snapshot), old)
                    .unwrap()
            );
            assert!(
                restarted
                    .reader()
                    .unwrap()
                    .idle_nudge_jobs_quiet("root", false, None, old)
                    .unwrap()
            );
            restarted.set_archived("root", true).unwrap();
            restarted.delete_session("root").unwrap();
            assert!(
                sql::job_first_seen(
                    &reader.conn,
                    sql::JobFirstSeenParams {
                        session: "root",
                        job_id: "preview"
                    }
                )
                .unwrap()
                .is_empty()
            );
        }
    }
}
