//! Scheduled and recurring prompts (ADR 0022): planning rules, the scheduler, and the
//! RFC 3339 helpers they need.
use crate::app::{ApiError, App};
use boblm_store::{NewSchedule, Schedule, ScheduleOutcome};
use rmcp::schemars;
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Weak},
    time::{SystemTime, UNIX_EPOCH},
};

const MS: i64 = 1000;
const MINUTE_MS: i64 = 60 * MS;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn millis(duration: std::time::Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

// Days since 1970-01-01 for a proleptic Gregorian date, and back (Howard Hinnant's algorithms).
fn days_from_civil(mut year: i64, month: i64, day: i64) -> i64 {
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// Parse an RFC 3339 timestamp (`2026-10-22T10:37:19Z`, with optional fraction and offset)
/// into UTC milliseconds.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = bytes.get(range)?;
        if digits.iter().all(u8::is_ascii_digit) {
            std::str::from_utf8(digits).ok()?.parse().ok()
        } else {
            None
        }
    };
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if civil_from_days(days) != (year, month, day) {
        return None; // such as February 30
    }
    let mut index = 19;
    let mut fraction_ms = 0;
    if bytes.get(index) == Some(&b'.') {
        let start = index + 1;
        let mut end = start;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == start {
            return None;
        }
        let digits = std::str::from_utf8(&bytes[start..end]).ok()?;
        let padded = format!("{:0<3}", &digits[..digits.len().min(3)]);
        fraction_ms = padded.parse::<i64>().ok()?;
        index = end;
    }
    let offset_ms = match bytes.get(index)? {
        b'Z' | b'z' if index + 1 == bytes.len() => 0,
        sign @ (b'+' | b'-') if index + 6 == bytes.len() && bytes[index + 3] == b':' => {
            let (h, m) = (number(index + 1..index + 3)?, number(index + 4..index + 6)?);
            if h > 23 || m > 59 {
                return None;
            }
            let total = (h * 60 + m) * MINUTE_MS;
            if *sign == b'+' { total } else { -total }
        }
        _ => return None,
    };
    Some(((days * 24 + hour) * 60 + minute) * MINUTE_MS + second * MS + fraction_ms - offset_ms)
}

/// UTC milliseconds as `2026-10-22T10:37:19Z`.
pub fn format_rfc3339(ms: i64) -> String {
    let seconds = ms.div_euclid(MS);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60
    )
}

/// What a caller asks for. Exactly one of `at` and `inMinutes` names the first run of a
/// one-shot; a repeating schedule may name one or start an interval from now.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ScheduleArgs {
    /// Fresh UUID for this schedule. Reuse it only to retry the identical call.
    pub schedule_id: String,
    /// The text to send to this chat when the schedule fires.
    pub prompt: String,
    /// First run, as an RFC 3339 timestamp such as 2026-10-22T14:30:00-04:00.
    pub at: Option<String>,
    /// First run, this many minutes from now.
    pub in_minutes: Option<u64>,
    /// Repeat every this many minutes (at least 1). Omit for a one-shot.
    pub every_minutes: Option<u64>,
    /// Stop repeating after this RFC 3339 time.
    pub end_at: Option<String>,
    /// Stop repeating after this many runs.
    pub max_runs: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub interval_ms: Option<i64>,
    pub next_run_at: i64,
    pub ends_at: Option<i64>,
    pub max_runs: Option<i64>,
}

fn minutes_text(minutes: i64) -> String {
    match minutes {
        m if m % (24 * 60) == 0 => format!("{} day{}", m / 1440, if m == 1440 { "" } else { "s" }),
        m if m % 60 == 0 => format!("{} hour{}", m / 60, if m == 60 { "" } else { "s" }),
        m => format!("{m} minute{}", if m == 1 { "" } else { "s" }),
    }
}

/// Validate a request into concrete times, or say exactly what to change.
pub fn plan(args: &ScheduleArgs, now: i64) -> Result<Plan, String> {
    let horizon = millis(boblm_tuning::MAX_SCHEDULE_HORIZON);
    let min_interval = millis(boblm_tuning::MIN_SCHEDULE_INTERVAL);
    let limit = boblm_tuning::MAX_MESSAGE_CHARS as usize - 200; // room for the heading
    let length = args.prompt.chars().count();
    if args.prompt.trim().is_empty() || length > limit {
        return Err(format!(
            "prompt must be between 1 and {limit} characters of text; this one is {length}."
        ));
    }
    let when = |text: &str, name: &str| {
        parse_rfc3339(text).ok_or_else(|| {
            format!(
                "{name} must be an RFC 3339 timestamp like 2026-10-22T14:30:00-04:00 or 2026-10-22T18:30:00Z; got {}.",
                boblm_protocol::client::shown(text)
            )
        })
    };
    let interval_ms = match args.every_minutes {
        None => None,
        Some(0) => {
            return Err(format!(
                "everyMinutes must be at least {} (the shortest repeat interval); got 0.",
                min_interval / MINUTE_MS
            ));
        }
        Some(minutes) => {
            let ms = i64::try_from(minutes)
                .ok()
                .and_then(|m| m.checked_mul(MINUTE_MS))
                .filter(|ms| *ms <= horizon)
                .ok_or_else(|| {
                    format!(
                        "everyMinutes {minutes} is longer than the {} day limit on how far ahead a schedule may reach.",
                        horizon / MINUTE_MS / 1440
                    )
                })?;
            if ms < min_interval {
                return Err(format!(
                    "everyMinutes must be at least {}; got {minutes}.",
                    min_interval / MINUTE_MS
                ));
            }
            Some(ms)
        }
    };
    if args.at.is_some() && args.in_minutes.is_some() {
        return Err("Give either at or inMinutes for the first run, not both.".into());
    }
    let first = match (&args.at, args.in_minutes, interval_ms) {
        (Some(at), _, _) => when(at, "at")?,
        (None, Some(minutes), _) => now.saturating_add(
            i64::try_from(minutes)
                .unwrap_or(i64::MAX)
                .saturating_mul(MINUTE_MS),
        ),
        (None, None, Some(interval)) => now + interval,
        (None, None, None) => {
            return Err("A one-shot schedule needs at (a time) or inMinutes (how long from now); add everyMinutes to repeat.".into());
        }
    };
    if first <= now {
        return Err(format!(
            "The first run, {}, is not in the future (it is {} now). Pick a later time.",
            format_rfc3339(first),
            format_rfc3339(now)
        ));
    }
    if first > now + horizon {
        return Err(format!(
            "The first run, {}, is more than {} days away, the furthest a schedule may reach.",
            format_rfc3339(first),
            horizon / MINUTE_MS / 1440
        ));
    }
    if interval_ms.is_none() && (args.end_at.is_some() || args.max_runs.is_some()) {
        return Err("endAt and maxRuns only apply to a repeating schedule; add everyMinutes, or remove them.".into());
    }
    let ends_at = match &args.end_at {
        None => None,
        Some(text) => {
            let end = when(text, "endAt")?;
            if end <= first {
                return Err(format!(
                    "endAt {} is not after the first run {}, so the schedule would never fire.",
                    format_rfc3339(end),
                    format_rfc3339(first)
                ));
            }
            Some(end)
        }
    };
    if args.max_runs == Some(0) {
        return Err("maxRuns must be at least 1; got 0.".into());
    }
    Ok(Plan {
        interval_ms,
        next_run_at: first,
        ends_at,
        max_runs: args.max_runs.map(i64::from),
    })
}

/// The message ID for one occurrence: stable, so a crash between sending and recording can
/// never deliver it twice.
pub fn message_id(schedule_id: &str, occurrence: i64) -> String {
    derived_uuid(&format!("boblm schedule\0{schedule_id}\0{occurrence}"))
}

/// A UUID derived from `seed`, the same every time, for messages boblm itself sends.
pub(crate) fn derived_uuid(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).hyphenated().to_string()
}

/// What the chat and the agent see.
pub fn prompt_text(schedule: &Schedule) -> String {
    let heading = match schedule.interval_ms {
        Some(ms) => format!(
            "{} (every {}):",
            boblm_runner::SCHEDULED_PREFIX,
            minutes_text(ms / MINUTE_MS)
        ),
        None => format!("{}:", boblm_runner::SCHEDULED_PREFIX),
    };
    format!("{heading}\n\n{}", schedule.prompt)
}

/// The next run after `occurrence`, or None when the schedule is finished. Occurrences that
/// have already passed are not replayed.
pub fn next_after(schedule: &Schedule, occurrence: i64, now: i64, fired: bool) -> Option<i64> {
    let interval = schedule.interval_ms?;
    if fired
        && schedule
            .max_runs
            .is_some_and(|max| schedule.runs + 1 >= max)
    {
        return None;
    }
    let mut next = occurrence + interval;
    if next <= now {
        next = occurrence + ((now - occurrence) / interval + 1) * interval;
    }
    if schedule.ends_at.is_some_and(|end| next > end) {
        return None;
    }
    Some(next)
}

pub fn view(schedule: &Schedule) -> serde_json::Value {
    let prompt: String = schedule.prompt.chars().take(200).collect();
    serde_json::json!({
        "scheduleId": schedule.id,
        "kind": if schedule.interval_ms.is_some() { "repeating" } else { "once" },
        "prompt": prompt,
        "promptTruncated": schedule.prompt.chars().count() > 200,
        "state": schedule.state,
        "nextRunAt": (schedule.state == "active").then(|| format_rfc3339(schedule.next_run_at)),
        "everyMinutes": schedule.interval_ms.map(|ms| ms / MINUTE_MS),
        "endAt": schedule.ends_at.map(format_rfc3339),
        "maxRuns": schedule.max_runs,
        "runs": schedule.runs,
        "skipped": schedule.skipped,
        "lastRunAt": schedule.last_run_at.map(format_rfc3339),
    })
}

impl App {
    pub(crate) async fn blocking<T: Send + 'static>(
        &self,
        doing: &str,
        work: impl FnOnce(boblm_store::Store) -> boblm_store::Result<T> + Send + 'static,
    ) -> Result<T, String> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || work(store))
            .await
            .map_err(|e| format!("Could not {doing}: {e}."))?
            .map_err(|e| format!("Could not {doing}: {e}."))
    }

    /// Create a schedule for `caller`'s own chat, or return the identical existing one.
    pub async fn schedule_prompt(
        &self,
        caller: &str,
        args: ScheduleArgs,
        now: i64,
    ) -> Result<serde_json::Value, String> {
        let id = boblm_runner::canonical_uuid(&args.schedule_id).map_err(|_| {
            format!(
                "scheduleId must be a fresh UUID like 123e4567-e89b-12d3-a456-426614174000; got {}.",
                boblm_protocol::client::shown(&args.schedule_id)
            )
        })?;
        let mut canonical = args.clone();
        canonical.schedule_id = id.clone();
        let arguments = serde_json::to_string(&canonical).map_err(|e| e.to_string())?;
        // An identical retry returns the schedule that exists, even when its time has passed
        // since the first call; only a new schedule is checked against the clock.
        let existing = {
            let (id, caller, arguments) = (id.clone(), caller.to_owned(), arguments.clone());
            self.blocking("look up the schedule", move |store| {
                Ok(store
                    .reader()?
                    .schedule(&id)?
                    .filter(|s| s.session == caller && s.arguments == arguments))
            })
            .await?
        };
        if let Some(schedule) = existing {
            let mut shown = view(&schedule);
            shown["alreadyExisted"] = true.into();
            return Ok(shown);
        }
        let plan = plan(&args, now)?;
        let (caller, prompt) = (caller.to_owned(), args.prompt.clone());
        let created_id = id.clone();
        let (schedule, created) = self
            .blocking("create the schedule", move |store| {
                store.create_schedule(
                    NewSchedule {
                        id: &created_id,
                        session: &caller,
                        prompt: &prompt,
                        arguments: &arguments,
                        interval_ms: plan.interval_ms,
                        next_run_at: plan.next_run_at,
                        ends_at: plan.ends_at,
                        max_runs: plan.max_runs,
                        created_at: now,
                    },
                    boblm_tuning::MAX_SCHEDULES_PER_CHAT,
                )
            })
            .await?;
        let mut shown = view(&schedule);
        shown["alreadyExisted"] = (!created).into();
        Ok(shown)
    }

    pub async fn list_schedules(&self, caller: &str) -> Result<serde_json::Value, String> {
        let caller = caller.to_owned();
        let schedules = self
            .blocking("list the schedules", move |store| {
                store.reader()?.schedules(&caller)
            })
            .await?;
        Ok(serde_json::json!({"schedules": schedules.iter().map(view).collect::<Vec<_>>()}))
    }

    pub async fn cancel_schedule(
        &self,
        caller: &str,
        schedule_id: &str,
    ) -> Result<serde_json::Value, String> {
        let id = boblm_runner::canonical_uuid(schedule_id).map_err(|_| {
            format!(
                "scheduleId must be a UUID from list_schedules; got {}.",
                boblm_protocol::client::shown(schedule_id)
            )
        })?;
        let (caller, shown_id) = (caller.to_owned(), id.clone());
        let _claim = self.schedule_claim.lock().await;
        let removed = self
            .blocking("cancel the schedule", move |store| {
                store.delete_schedule(&id, &caller)
            })
            .await?;
        if removed {
            Ok(serde_json::json!({"scheduleId": shown_id, "cancelled": true}))
        } else {
            Err(format!(
                "This chat has no schedule {shown_id}; it may already be cancelled or belong to another chat. Call list_schedules to see this chat's schedules."
            ))
        }
    }

    async fn chat_is_busy(&self, session: &str) -> Option<bool> {
        let status = self.handle(session).await.ok()?.status().await.ok()?;
        // A held queue with nothing queued is just a fresh start, not a busy chat; a chat the
        // owner stopped is left alone.
        Some(
            status.turn_active
                || status.stopping
                || status.queued > 0
                || status.dispatching > 0
                || status.stopped,
        )
    }

    /// Settle and send everything due at `now`. Returns how many schedules it acted on.
    pub async fn run_due_schedules(&self, now: i64) -> Result<usize, ApiError> {
        let due = self
            .blocking("look for due schedules", move |store| {
                store.reader()?.due_schedules(now)
            })
            .await
            .map_err(ApiError::internal)?;
        let mut acted = 0;
        for schedule in due {
            if self.settle_due(schedule, now).await? {
                acted += 1;
            }
        }
        Ok(acted)
    }

    /// Act on one due occurrence. Cancelling takes the same lock, so a cancel that succeeds
    /// means the occurrence was never sent, and one that waits finds it already sent.
    pub async fn settle_due(&self, schedule: Schedule, now: i64) -> Result<bool, ApiError> {
        let _claim = self.schedule_claim.lock().await;
        let occurrence = schedule.next_run_at;
        let still_due = {
            let id = schedule.id.clone();
            self.blocking("recheck the due schedule", move |store| {
                store.reader()?.schedule(&id)
            })
            .await
            .map_err(ApiError::internal)?
        };
        if !still_due.is_some_and(|s| s.state == "active" && s.next_run_at == occurrence) {
            return Ok(false); // cancelled or settled since the list was read
        }
        let overdue = now - occurrence;
        let Ok(handle) = self.handle(&schedule.session).await else {
            return Ok(false); // the chat is not running in this server; try again next tick
        };
        let outcome = match schedule.interval_ms {
            None if overdue > millis(boblm_tuning::SCHEDULE_MISSED_GRACE) => {
                ScheduleOutcome::Missed
            }
            Some(_)
                if overdue > millis(boblm_tuning::SCHEDULE_ON_TIME_SLACK)
                    || self.chat_is_busy(&schedule.session).await != Some(false) =>
            {
                ScheduleOutcome::Skipped {
                    next: next_after(&schedule, occurrence, now, false),
                }
            }
            _ => {
                // A one-shot is sent whatever the chat is doing; it queues for idle.
                let id = message_id(&schedule.id, occurrence);
                if let Err(error) = handle.send_automated(&id, &prompt_text(&schedule)).await {
                    eprintln!(
                        "server: could not send scheduled prompt {}: {error}",
                        schedule.id
                    );
                    return Ok(false);
                }
                ScheduleOutcome::Fired {
                    next: next_after(&schedule, occurrence, now, true),
                }
            }
        };
        let schedule_id = schedule.id.clone();
        self.blocking("record the schedule's run", move |store| {
            store.advance_schedule(&schedule_id, occurrence, outcome)
        })
        .await
        .map_err(ApiError::internal)?;
        Ok(true)
    }

    /// Start the background task that fires due prompts. It ends when the app is dropped.
    pub fn start_scheduler(self: &Arc<Self>) {
        self.start_notifications();
        let app: Weak<App> = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            let mut subagents = Some(crate::subagent_wake::Observer::new());
            loop {
                let Some(app) = app.upgrade() else { break };
                if let Err(error) = app.run_due_schedules(now_ms()).await {
                    eprintln!("server: scheduler: {error}");
                }
                app.sweep_secrets(now_ms()).await;
                app.deliver_question_answers().await;
                app.wake_completed_subagents(&mut subagents).await;
                app.deliver_subagent_reports().await;
                if let Err(error) = app.run_idle_nudges(now_ms()).await {
                    eprintln!(
                        "Idle nudges could not be checked: {error}. The server will check again on its next tick."
                    );
                }
                if let Err(error) = app.sweep_subagent_lifetime(now_ms()).await {
                    eprintln!("server: subagent lifetime: {error}");
                }
                drop(app);
                tokio::time::sleep(boblm_tuning::SCHEDULER_TICK).await;
            }
        });
        self.keep_scheduler(task);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_792_000_000_000; // 2026-10-14T...Z, any fixed instant

    fn args(patch: impl FnOnce(&mut ScheduleArgs)) -> ScheduleArgs {
        let mut args = ScheduleArgs {
            schedule_id: "123e4567-e89b-12d3-a456-426614174000".into(),
            prompt: "keep going".into(),
            at: None,
            in_minutes: Some(10),
            every_minutes: None,
            end_at: None,
            max_runs: None,
        };
        patch(&mut args);
        args
    }

    #[test]
    fn timestamps_round_trip_and_honor_offsets_and_reject_nonsense() {
        let zulu = parse_rfc3339("2026-10-22T10:37:19Z").unwrap();
        assert_eq!(format_rfc3339(zulu), "2026-10-22T10:37:19Z");
        assert_eq!(parse_rfc3339("2026-10-22T06:37:19-04:00"), Some(zulu));
        assert_eq!(parse_rfc3339("2026-10-22T12:07:19+01:30"), Some(zulu));
        assert_eq!(parse_rfc3339("2026-10-22T10:37:19.500Z"), Some(zulu + 500));
        assert_eq!(parse_rfc3339("2026-10-22t10:37:19z"), Some(zulu));
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            parse_rfc3339("2024-02-29T00:00:00Z")
                .map(format_rfc3339)
                .as_deref(),
            Some("2024-02-29T00:00:00Z")
        );
        for bad in [
            "2026-02-30T10:00:00Z",
            "2026-13-01T10:00:00Z",
            "2026-10-22 10:37:19Z",
            "2026-10-22T10:37:19",
            "2026-10-22T25:00:00Z",
            "2026-10-22T10:37:19+0100",
            "tomorrow",
            "",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_plan_names_the_first_run_and_the_ways_it_can_end() {
        let p = plan(&args(|_| {}), NOW).unwrap();
        assert_eq!(
            p,
            Plan {
                interval_ms: None,
                next_run_at: NOW + 600_000,
                ends_at: None,
                max_runs: None
            }
        );
        let p = plan(
            &args(|a| {
                a.in_minutes = None;
                a.every_minutes = Some(30);
                a.max_runs = Some(3);
            }),
            NOW,
        )
        .unwrap();
        assert_eq!(
            (p.interval_ms, p.next_run_at, p.max_runs),
            (Some(1_800_000), NOW + 1_800_000, Some(3))
        );
        let end = format_rfc3339(NOW + 10_000_000);
        let p = plan(
            &args(|a| {
                a.every_minutes = Some(5);
                a.end_at = Some(end.clone());
            }),
            NOW,
        )
        .unwrap();
        assert_eq!(p.ends_at, Some(parse_rfc3339(&end).unwrap()));
    }

    #[test]
    fn a_bad_plan_says_what_to_change() {
        let fail = |patch: fn(&mut ScheduleArgs)| plan(&args(patch), NOW).unwrap_err();
        assert!(fail(|a| a.prompt = "  ".into()).contains("prompt must be between 1 and"));
        assert!(fail(|a| a.every_minutes = Some(0)).contains("everyMinutes must be at least 1"));
        assert!(
            fail(|a| {
                a.in_minutes = None;
            })
            .contains("needs at (a time) or inMinutes")
        );
        assert!(
            fail(|a| a.at = Some("2026-10-22T10:00:00Z".into()))
                .contains("Give either at or inMinutes")
        );
        assert!(
            fail(|a| {
                a.in_minutes = None;
                a.at = Some("next tuesday".into());
            })
            .contains("RFC 3339 timestamp like")
        );
        assert!(
            fail(|a| {
                a.in_minutes = None;
                a.at = Some("2001-01-01T00:00:00Z".into());
            })
            .contains("not in the future")
        );
        assert!(fail(|a| a.in_minutes = Some(60 * 24 * 400)).contains("days away"));
        assert!(fail(|a| a.max_runs = Some(2)).contains("only apply to a repeating schedule"));
        assert!(
            fail(|a| {
                a.every_minutes = Some(5);
                a.max_runs = Some(0);
            })
            .contains("maxRuns must be at least 1")
        );
        assert!(
            fail(|a| {
                a.every_minutes = Some(5);
                a.end_at = Some(format_rfc3339(NOW + 1000));
            })
            .contains("never fire")
        );
    }

    #[test]
    fn occurrences_have_stable_distinct_ids() {
        let a = message_id("sched", 1000);
        assert_eq!(a, message_id("sched", 1000));
        assert_ne!(a, message_id("sched", 2000));
        assert_ne!(a, message_id("other", 1000));
        assert!(boblm_runner::canonical_uuid(&a).is_ok());
    }

    fn repeating(runs: i64, max_runs: Option<i64>, ends_at: Option<i64>) -> Schedule {
        Schedule {
            id: "s".into(),
            session: "chat".into(),
            prompt: "go".into(),
            arguments: "{}".into(),
            interval_ms: Some(60_000),
            next_run_at: 1_000_000,
            ends_at,
            max_runs,
            runs,
            skipped: 0,
            last_run_at: None,
            state: "active".into(),
            created_at: 0,
        }
    }

    #[test]
    fn the_next_run_skips_the_past_and_stops_at_the_end() {
        let s = repeating(0, None, None);
        assert_eq!(next_after(&s, 1_000_000, 1_000_000, true), Some(1_060_000));
        // Down for ten minutes: resume at the first future slot, no catch-up.
        assert_eq!(next_after(&s, 1_000_000, 1_600_000, false), Some(1_660_000));
        assert_eq!(next_after(&s, 1_000_000, 1_630_000, false), Some(1_660_000));
        assert_eq!(
            next_after(&repeating(2, Some(3), None), 1_000_000, 1_000_000, true),
            None
        );
        assert_eq!(
            next_after(&repeating(2, Some(3), None), 1_000_000, 1_000_000, false),
            Some(1_060_000)
        );
        assert_eq!(
            next_after(
                &repeating(0, None, Some(1_050_000)),
                1_000_000,
                1_000_000,
                true
            ),
            None
        );
        let mut once = repeating(0, None, None);
        once.interval_ms = None;
        assert_eq!(next_after(&once, 1_000_000, 1_000_000, true), None);
    }

    #[test]
    fn the_heading_says_where_a_prompt_came_from() {
        let mut s = repeating(0, None, None);
        assert_eq!(prompt_text(&s), "Scheduled prompt (every 1 minute):\n\ngo");
        s.interval_ms = Some(90 * 60_000);
        assert!(prompt_text(&s).starts_with("Scheduled prompt (every 90 minutes):"));
        s.interval_ms = Some(2 * 3_600_000);
        assert!(prompt_text(&s).starts_with("Scheduled prompt (every 2 hours):"));
        s.interval_ms = None;
        assert_eq!(prompt_text(&s), "Scheduled prompt:\n\ngo");
    }
}
