---
# boblm-dv29
title: 'Antigravity agent: Gemini chats through agy'
status: todo
type: epic
created_at: 2026-11-20T13:56:48Z
updated_at: 2026-11-20T13:56:48Z
parent: boblm-fava
---

Add Google's Antigravity CLI (`agy`) as a fourth agent so chats can run Gemini models. The owner may later make it the second plan account in place of Claude plan B; no routing change is decided.

**Deferred (2026-11-20):** revisit when the owner's current subscriptions run out. Nothing is installed and no code has changed. The full research, with sources, is in `docs/research/antigravity-cli.md`.

## Decisions so far

- **Transport: `agy acp`** (Agent Client Protocol, JSON-RPC over stdio), not headless `--input-format stream-json`. ACP documents mid-turn steering, `session/cancel`, session load/resume/list/close, model and thinking-level session options, permission requests sent to the client, and per-session `mcpServers` (HTTP with headers, which needs `--process-per-session`). Headless stream-json has no interrupt, can't take a per-session MCP server, and ends the session on CLI slash commands such as `/usage` and `/model`.
- **Catalog:** three models, in the same tiers as Haiku/Sonnet/Opus and Luna/Sol/Astra:
  - `hydrogen` (Gemini 4 Hydrogen): small, fast; efforts low, medium, high, xhigh (default high).
  - `neon` (Gemini 4 Neon): daily driver, said to be strong at coding; efforts low, medium, high, xhigh (default high); context up to 512K tokens.
  - `carbon` (Gemini 4 Carbon): top tier, Ultra-only; efforts low, medium, high.
  - ACP shows xhigh as "Extra". The driver maps BobLM's `xhigh` to it.
- **Left out:** Argon is the oldest Gemini 4 model (2026-09-30, before Carbon on 11-10 and Neon on 11-18) and has no role next to Carbon and Neon. Orchestrator (a Neon or Carbon planner handing execution to Hydrogen inside one turn, fixed effort) hides model switches from BobLM's role routing (ADR 0043); it's worth one side-by-side test later. Claude and GPT-OSS through Antigravity are left out because BobLM reaches Claude directly, and the 4.6 models and GPT-OSS go away on 2026-11-30.
- **Plan:** Google AI Ultra if the owner subscribes. Carbon and Orchestrator (Carbon) are Ultra-only.
- **Secrets stay in SOPS.** A Linux Secret Service is not a replacement. On this headless host it would either be auto-unlocked (an empty password, so effectively plaintext on disk) or need a manual unlock after every reboot, and once unlocked any `dev` process can read every secret over D-Bus. That's no better than the age key in `~/.config/sops/age/keys.txt` (mode 600). SOPS also gives portable encrypted backups, needs no daemon, and already backs `request_sops_secret` (ADR 0025) and one-key spawn injection (ADR 0026). The real gap is that agents run as the user who owns the secrets; only OS isolation (agents as a separate user) closes it.
- **`agy` credentials, cheapest option that works:**
  1. If `agy` falls back to a token file without a keyring, use the file.
  2. If it requires a keyring, run a small auto-unlocked Secret Service just for `agy` (needs owner approval: it's a NixOS config change), accepting that it's no more secure than a file.
  3. A Gemini API key (`modelProvider: "gemini"` plus `GEMINI_API_KEY` from SOPS) works headless but bills per token, not against the plan allowance.

## Facts gathered

- **Install:** `curl -fsSL https://antigravity.google/cli/install.sh | bash -s -- --skip-path --skip-aliases` puts the binary at `~/.local/bin/agy`. It needs glibc 2.28 or later. This host is NixOS with glibc 2.42 and nix-ld enabled, and Claude's native binary already runs from `~/.local/bin`.
- **Sign-in:** `agy acp --login` signs in over SSH by pasting a code. `agy` stores the OAuth token in the OS keyring. This host's user D-Bus has no Secret Service provider (no gnome-keyring, no KWallet). The docs don't say whether `agy` falls back to a file.
- **Config:** `~/.gemini/antigravity-cli/settings.json`; logs in `~/.gemini/antigravity-cli/log/`. No documented environment variable moves the config directory, so isolating two accounts the way `CLAUDE_CONFIG_DIR` and `CODEX_HOME` do is unproven.
- **Unattended settings:** `--dangerously-skip-permissions` (or allow every ACP permission request), plus `artifactReviewPolicy: "always-proceed"`. The default `asks-for-review` waits for the owner on implementation plans. Also use `--disable-telemetry`. Questions show in chat for clients without ACP elicitation, which fits ADR 0010.
- **Quotas:** five-hour and weekly limits per model group (Gemini models; Claude and GPT models), not per account. `/usage` (alias `/quota`) is a TUI panel; `agy -p /usage` probably prints a text report in an undocumented format. It starts no inference turn, so it fits ADR 0037's telemetry rule.
- **Compaction:** no `/compact` appears in the CLI command list. It may be automatic only, which would change `/compact` behavior (ADR 0019) for this agent.

## Children

- `boblm-pe01`: install, sign in, and record the live protocol. The free plan (Gemini 3.8 Flash) is enough to check credential storage, ACP shapes, MCP headers, steering and cancel before paying for Ultra. The quota output and the Carbon and Neon effort lists need the real plan.
- `boblm-gswe`: ADR and `antigravity-driver` crate. Blocked by pe01.
- `boblm-rdaj`: accounts and quota meters. Blocked by pe01.

## Open owner items

1. Install `agy` and sign in, or let the agent run the installer while the owner does only the sign-in.
2. Approve a headless Secret Service in the NixOS config if `agy` requires one.
