# 0048: Subagent grading

A parent can grade a subagent's result with the `grade_subagent` MCP tool, so the owner
collects tuning data: which model, effort and routing step produce good or poor work. The
feature is temporary. It is expected to be disabled or removed once the owner has enough
data (see Kill switch).

## Tool

`grade_subagent(sessionIdPrefix, rating, note?)` takes a full session ID or unique prefix,
a `rating` of exactly `good`, `ok` or `poor`, and an optional plain-text `note` of at most
`MAX_GRADE_NOTE_CHARS` (500) characters, counted as characters and trimmed. An empty note is
stored as no note. Any role can be graded. The tool description tells agents to grade from
the result they already have, never to re-read code or re-verify findings just to grade, to
skip when unsure, and to give implementers a quick gut rating only.

The caller must be the chat's `parent_session_id`, its `root_session_id`, or the chat that
spawned it (`peer_spawn_requests.caller`). Anyone else is refused with a plain sentence. The
check runs in the same transaction that writes the grade. An archived subagent can be
graded, which matters because parents usually archive first. A deleted chat cannot, because
it no longer exists; grade before deleting.

Grading again overwrites: there is one row per graded subagent, keeping the original
`created_at` and moving `updated_at`.

## Blind reviews

Reviews are graded blind (ADR 0043). The tool result is only
`{sessionId, rating, replaced}`. Success results and refusals never name the subagent's
model, effort, step, provider or account. The caller only chose the session and rating, so
echoing the rating discloses nothing.

## Storage

Migration 056 creates `subagent_grades`: `session` (primary key), `grader` (the calling
session), `rating` (CHECK for the three values), `note`, `role`, `agent`, `model`, `effort`,
`step`, `account_key`, `title`, `created_at` and `updated_at`. At grading time the store
copies model, effort, agent, account and title from the session row, and the step from the
stored spawn receipt's resolved JSON (null when there is no receipt). The copy keeps the
data meaningful after the model of a chat changes or the chat is gone.

The table has no foreign keys. Chat deletion (ADR 0030) does not touch it, so a grade
survives deleting the chat it describes. SQL lives in `crates/store/src/grades.sql` beside
`grades.rs`, with Marmot-generated bindings.

## Reviewer hint

A finished reviewer's result report to its parent ends its body with one line: "Optional:
grade this review with grade_subagent (good, ok or poor)." The line names no model or
provider. Implementer and planner reports, error reports and rate-limit reports carry no
hint; those can still be graded on the parent's own initiative.

## Kill switch

`SUBAGENT_GRADING_ENABLED` in `crates/tuning` is true. When false the tool refuses with
"Subagent grading is turned off.", the hint is not appended, and stored grades stay
untouched. Routing, caps, notifications and blind-review behavior do not depend on it.

## Report

`bin/grade-report` is a read-only script (bash and `sqlite3 -readonly`, database from
`BOBLM_DB` or `~/.local/share/boblm/boblm.sqlite`, with a busy timeout). It prints the total
and counts of good, ok and poor grouped by role, model, effort and step. `--notes` also
prints each note with its rating, role, model and effort. It prints no chat content.
