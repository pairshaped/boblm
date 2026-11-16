# 0052: Claude update job

BobLM launches every Claude child with the CLI's own updater disabled (ADR 0051), so a nightly
job, `bin/claude-update-check`, is the only thing that updates the installed CLI. It installs
new versions automatically and silently: no notifications, no prompts.

## What it does

A systemd user timer (`systemd/boblm-claude-update.timer`, about 03:40 with up to 20 minutes of
random delay, `Persistent=true`) runs `systemd/boblm-claude-update.service`, which runs the
script once. The script:

1. Takes a non-blocking lock next to the state file. A second run exits 0 and says so.
2. Reads the channel from `autoUpdatesChannel` in `~/.claude/settings.json` (`latest` or
   `stable`). A missing, unreadable or unexpected value means `latest`.
3. Fetches `https://downloads.claude.ai/claude-code-releases/<channel>` (30 second timeout,
   3 attempts). The body must be a plain `major.minor.patch` version. A network or parse failure
   is recorded and the run exits 0, because a transient outage is not an alarm.
4. Reads the installed version from `claude --version` and compares both as integer tuples.
5. Runs `claude update` (20 minute timeout) only when the channel offers a strictly newer
   version, then runs the smoke test.

The job never downgrades. When the channel is older than the installed version (`stable` can lag
`latest`), it records "up to date" with a note that the channel is behind, and changes nothing.

It only touches an install whose `claude` is a symlink into a directory named `versions` (the
native install layout). Any other layout is recorded as "unknown install layout" and left alone.

## Smoke test

After `claude update`, the new binary runs one real turn through the launch shape the driver
uses: stream-json in and out, `--permission-prompts none`, `--permission-mode bypassPermissions`,
`--model haiku`, `--prompt-suggestions true`, `--no-session-persistence`, the same
`--disallowedTools` list and the `CLAUDE_ENV` variables from `crates/tuning`. It sends one user
message and keeps stdin open until the result event arrives, then closes it. The run uses a
temporary working directory and a 120 second timeout.

It passes only when all of these hold: the process exits 0; every stdout line is valid JSON;
there is a `system` `init` event; there is a `result` event with subtype `success`, `is_error`
false and text containing "ok" (any case); and there is a `rate_limit_event`.

## Rollback and failed-version memory

If `claude update` fails, times out, or leaves `--version` different from the offered version,
the job restores the previous symlink target when the update changed it. If the smoke test
fails, it always restores the previous target. Restoring creates a temporary symlink beside the
real one and renames it over it, then checks that `--version` reports the previous version again.

A smoke failure stores the version in `failedVersion` in the state file. The job skips that
exact version on later runs; a newer version is tried normally. An update failure is retried the
next night.

## Silence and where to look

Nothing is sent anywhere. Every outcome prints one timestamped line to stdout, which the journal
keeps, and updates the state file `~/.local/state/boblm/claude-update.json` (directory 0700,
file 0600, written atomically). The state holds `checkedAt`, `channel`, `installed`,
`available`, `result`, `previous`, `failedVersion`, `lastSuccessAt` and `note`.

    journalctl --user -u boblm-claude-update
    cat ~/.local/state/boblm/claude-update.json

The job exits 1 after an update or smoke failure, so `systemctl --user status
boblm-claude-update` shows the unit as failed. The job never prints the environment, tokens or
settings contents.

## Owner settings

The job reads `autoUpdatesChannel` and the CLI reads `minimumVersion` from
`~/.claude/settings.json`; the CLI skips update targets below `minimumVersion`. The job never
writes that file.

## Effect on chats

A new BobLM chat, or a chat whose child restarts, launches the installed version at that moment.
Running chats keep the process they have until it ends.

Old versions in `versions/` are pruned by the CLI's installer, not by this job (the installer is
expected to prune; check the folder after the first update).

## Known drift risk

The smoke turn mirrors the driver's flags by hand. `bin/test-claude-update-check` checks that the
script's environment list, disallowed tools and permission mode equal `crates/tuning`, but a new
driver flag is not caught. When `spawn_command` changes its arguments, update the smoke test too.

## Install and remove

    bin/install-claude-update-timer
    bin/install-claude-update-timer --remove

`bin/claude-update-check --dry-run` fetches and compares and prints what it would do, without
updating, rolling back, running the smoke turn or writing state.
