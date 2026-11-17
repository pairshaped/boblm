# 0053: Database backups

`bin/backup-db` creates a verified online copy of the whole SQLite database, including
chats, messages and events, stored as a single self-contained file in DELETE journal mode. SQLite's online backup API includes committed WAL data while
BobLM keeps writing. A 60 second busy timeout accommodates database contention.

## Schedule and retention

The systemd user timer runs nightly at 03:15 local time with up to 10 minutes of random
delay. `Persistent=true` catches a missed run after login. Install it with
`bin/install-backup-timer`; disable and remove it with `bin/install-backup-timer --remove`.

The script takes a non-blocking lock beside its state file. It checks available disk space
against the database size plus WAL size plus 2 GiB, writes a private `.partial` file,
checks `PRAGMA integrity_check` and the sessions table, then atomically renames the copy.
Only then does it prune. A failed backup never deletes older backups.

Managed backups are regular files named `boblm-*.sqlite`, excluding symlinks and directories.
The newest N files by modification time survive (name breaks ties), with a slot reserved
for the new backup. The default is 3. Labeled backups such as `boblm-pre-NNN.sqlite` count
in the same pool. Pruning removes discarded backups' regular WAL and SHM sidecars, never
sidecars of retained files. Managed partial files older than one day are removed after a
successful backup, including orphan partial WAL, SHM and journal sidecars. A pruning failure keeps the new backup and exits 1.

Backups contain every chat present when taken, including chats deleted since then.
Files are 0600 in a 0700 directory. Retention bounds how long deleted chats persist in
these backups. Existing retained files are protected by the directory's permissions.

## Commands and configuration

Run `bin/backup-db --label pre-NNN` before deploying a change that adds migration NNN.
Labels accept letters, numbers, dots, underscores and hyphens; existing targets are refused.
The default name is `boblm-nightly-<UTC YYYYMMDDTHHMMSSZ>.sqlite`.

`bin/backup-db --dry-run` prints source, destination, space check and retention plan without
writing anything or taking a lock. `--keep N` overrides `BOBLM_BACKUP_KEEP` (default 3).
`BOBLM_DB` overrides the database path; otherwise the script reads only `database` from
`BOBLM_CONFIG` (default `~/.config/boblm/boblm.json`), falling back to
`~/.local/share/boblm/boblm.sqlite`. `BOBLM_BACKUP_DIR` defaults to
`~/.local/share/boblm/backups`. `BOBLM_BACKUP_MIN_FREE_BYTES` overrides the space threshold.

Read timestamped outcomes with `journalctl --user -u boblm-db-backup`. Successful runs
atomically write `~/.local/state/boblm/db-backup.json` (0600), overridden by
`BOBLM_BACKUP_STATE`: `finishedAt`, `file`, `sizeBytes`, `integrity`, `kept`, `deleted`,
and `freeBytesAfter`. A failure leaves the last successful state intact; check the journal
for current failures. No row contents or configuration secrets are logged.

## Restore

Rehearse restoration on a copy first. Restoring replaces all current database contents.

1. Stop BobLM and its session runners so nothing can write to the database.
2. Copy the chosen verified backup over the configured database file, preserving private permissions.
3. Remove the old database's `-wal` and `-shm` files.
4. Start BobLM. Database migrations apply forward on startup.
