# 0050: Codex kill tracing

## Design

An optional switch launches each new `codex app-server` under `strace` so the kernel records
who sends every kill signal. It exists to find the sender of an unexplained exit 143 (SIGTERM)
on a Codex agent's background command. It is off unless the server config names a directory:

```json
"codexKillTraceDir": "/home/dev/.local/share/boblm/kill-traces"
```

Add the field to `~/.config/boblm/boblm.json` and restart BobLM. There is no settings screen
(ADR 0018). The web server and every session runner read the config file at startup
(ADR 0040), so the value reaches the code that spawns Codex through `AgentFactory`. A chat
whose runner is already running keeps its old app-server and its old setting; it is traced
after its runner restarts.

## Launch

With the field set, the child is
`strace -f --seccomp-bpf -tt -e trace=<signal calls> -o <dir>/codex-kills-<chat id>-<unix seconds>.log codex app-server ...`
followed by the usual arguments. The program name, fixed arguments and file name pattern are
constants in `crates/tuning`. The command is built by `boblm_codex_driver::kill_trace`.
`--seccomp-bpf` requires `-f`, and strace 7.x rejects the unprefixed name `tgsigqueueinfo`,
so the list uses `rt_sigqueueinfo` and `rt_tgsigqueueinfo`.

The driver still puts the child in its own process group and kills the group on stop.
strace is the group leader and its tracee is in the group, so the group kill takes down both.

## Safe failure

The switch never stops a chat from starting. If the path is not absolute, `strace` is not on
PATH (a lookup, strace is not run), or the directory cannot be created (mode 0700), BobLM
prints one plain-English warning to stderr and launches Codex untraced.

## Reading a log

Each line is one signal-sending system call with a timestamp, the sender's pid, and the
target pid or process group, for example `1762944 10:46:10.975557 kill(1762945, SIGTERM) = 0`.
When a signal is delivered, strace also prints a `--- SIGTERM {... si_pid=<sender> ...} ---`
line. Take the process ids from the background job ids, then
`grep SIGTERM <log>` and look for those target pids; the leading pid is the sender, and a
negative target is a process group.

## Cost and caveats

The seccomp filter stops the tracee only for the listed calls, so overhead is small. Tracing
uses ptrace, which can conflict with Codex's own command sandbox. Verify on one chat that
commands still run before relying on it. BobLM never rotates or deletes the logs.
