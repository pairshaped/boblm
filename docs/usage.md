# Using BobLM

Open your server's HTTPS address and enter the bearer token printed by
`boblm init`. Projects and chats belong to the server, so you'll see the same ones
on your other devices. Each browser or installed app keeps its own connection
credentials, drafts and notification permission.

## Projects and chat names

Click or tap a project name to expand or collapse its chats. The caret points
down when expanded and right when collapsed.

To rename a project, press and hold its name. On desktop, you can also focus the
project name and press F2. A normal click only toggles the project.

Use the folder icon above the project list to add a project folder. The plus
beside a project creates a chat in it. To rename a chat, click or tap its name in
the chat header.

Archived chats and projects with no unarchived chats appear dimmed. A dim project
can still contain archived chats.

## Reorder projects and chats

With a mouse or trackpad, drag a project row to change the project order. Drag a
chat row above or below another chat to reorder it within the same project.
The drop marker shows where it will land. Dragging a chat doesn't move it into
a different project.

The order is saved on the server and shared across your devices. Reordering or
collapsing projects can change the chat numbers used by keyboard shortcuts.

## Chat numbers, colors and status

The small number beside a chat is its keyboard shortcut. Numbers 1 through 9
follow the sidebar order across expanded projects, skipping archived chats.
Collapsed projects aren't included, so expanding or collapsing one can change
which chat a number refers to.

| Platform         | Switch to a numbered chat |
| ---------------- | ------------------------- |
| Mac              | Command + number          |
| Windows or Linux | Alt + number              |

For example, Command + 3 on Mac or Alt + 3 on Windows opens the chat currently
marked 3. Chats beyond the first nine have no number shortcut. Archived chats
show a trash control in that space instead.

The number and the three-dot status indicator share the agent's color:

| Agent       | Color |
| ----------- | ----- |
| Claude Code | Peach |
| Codex       | Blue  |
| Pi          | Lavender |

The color identifies the agent family, not the exact model. Check the Model
selector beneath the composer for the selected model.

Animated dots mean the agent is working on a turn. Idle and archived chats have
no dots.

## Codex chat speed

Choose **Speed** beside **Effort** beneath the composer. **Normal** is standard
speed (1x). All Codex models in the catalog offer **Fast**. Sol (`gpt-6.2-sol`) also
offers **Ultrafast**. The selector appears only for models with a faster tier.

BobLM saves the choice per chat and applies it when the next turn starts. If
you switch to a model that doesn't support the selected speed, the footer resets
to **Normal**. A disabled Speed selector means the runner hasn't reported speed
support. After a server upgrade, an older runner may still be finishing work.
See [Restart one chat's runner](running.md#restart-one-chats-runner).

To start a peer or subagent at Fast, include `"speed": "fast"` alongside
`title` and `message` in the spawn arguments. Omit `speed` to start the child at
Normal, even when the parent is running at Fast.

## Context and weekly usage

On wider screens, two bars beneath the composer show how much context and weekly
allowance you've used. Both fill as usage increases. Pi shows only context.
The bars disappear on mobile and when the composer is too narrow.

Hover over Weekly for the more precise percentage, reset date/time in your local
timezone, and the observation time. Hover over Context for the token counts.
A dash means the agent hasn't reported a usable sample. After a weekly reset,
the old percentage stays unavailable until a fresh sample arrives. Opening a
chat doesn't start an agent just to fill these bars.

## The follow lock and the account usage clock

The small lock beneath the message scrollbar means the view is following new
messages. It's an indicator, not a button. Scroll up to stop following; the lock
disappears and new output won't pull you back down. Use the down-arrow button to
jump directly to the latest messages and follow them again.

The clock button above the project list opens Account usage: the allowance
remaining for each account, with reset times and how old each reading is. It
loads fresh readings each time you open it. BobLM has no Lock button: a browser
keeps its token until you clear the site's stored data.

## Moving a chat to the other Claude plan account

To move an idle Claude chat from one plan account to the other, keeping the same
conversation, model and effort, run `bin/move-chat-account <chat ID or unique prefix> <a|b>`
on the BobLM host. Add `--wait SECONDS` to wait for the chat to become idle (the default
refuses a busy chat), `--no-compact` to skip compaction, and `--dry-run` to see the plan
without changing anything.

The chat must have no running turn, queued message, background job or open subagent.
If its context is 90% full or more, the script compacts it first, because the first turn
on the new account pays for the whole context again. A move that fails keeps the chat on
its old account and says why. The chat can't return to the old account automatically
until that account's weekly reset.

## Reading long chats

The browser keeps a moving window of up to 500 timeline rows. Scrolling backward
loads older pages and drops newer rows from the window. Scrolling forward fetches
those newer pages again. Your full history remains on the server.

The limit counts rows before routine tool calls are grouped, so one collapsed
tool group can represent many retained rows. Tool-only pages look farther for
nearby conversation text, within that budget. Scrolling or using the explicit
load controls advances history; restoring a scroll position doesn't trigger more
page requests.

The down-arrow button fetches the latest page directly; it doesn't load every
message between your position and the bottom.

After fifteen minutes without a change in scroll position, BobLM clears the
rendered window, opens the latest page and follows new messages again. This also
applies while the app stays visible. If the app is hidden, it waits until you
return. Your unfinished draft survives the reset.

## The right drawer

Use the drawer button at the right of the chat header to open or close the chat's
details. It has two tabs:

- Questions contains prompts the agent wants you to answer. Click anywhere on a
  suggested answer to submit it immediately. Choose Type a different answer to
  write a response, then submit it. Discuss later leaves the question unanswered
  and asks the agent to revisit it after the other open prompts are resolved.
  The waiting count excludes the question currently shown.
- Agents lists the chat's subagents, with working agents above finished ones.
  Failed agents are marked, and each entry shows its start or finish time.
  Running agents have animated dots. Click an agent row to watch its conversation
  inside the drawer. Its title and a back arrow appear below the tabs. Use Back
  or click Agents again to return to the list; Questions remains available.
  The conversation is read-only. It follows new messages until you scroll up;
  use its down arrow to return to the latest output. The small lock indicates
  that it is following, just as in the main chat.

On a phone, the drawer fills the screen. Use its toggle beside the tabs to get
back to the chat. It starts closed unless you open a question from a notification.

## Install it as an app

Use the same HTTPS address that you normally open in the browser. Installing the
app adds a launcher for your server; the server still needs to be running and
reachable from the device.

### Mac: add it to the Dock

On macOS Sonoma 14 or later:

1. Open BobLM in Safari.
2. Choose File > Add to Dock.
3. Name the app and click Add.
4. Launch it from the Dock. If it asks for a token, enter the same server token.

See [Apple's Mac web-app instructions](https://support.apple.com/en-gb/104996).
Mac Safari can also receive web notifications without installing a Dock app;
installation gives BobLM its own app window.

### iPhone: add it to the Home Screen

1. Open BobLM in Safari.
2. Open Share, either directly or through the page menu, depending on your Safari
   layout.
3. Choose Add to Home Screen. If Open as Web App is offered, leave it on.
4. Tap Add, then launch BobLM using its new Home Screen icon.
5. If asked, enter the same server token.

For push notifications on iPhone, use the installed Home Screen app. This requires
iOS 16.4 or later. An ordinary Safari tab doesn't provide that notification path.
See [Apple's Home Screen instructions](https://support.apple.com/guide/iphone/open-as-web-app-iphea86e5236/ios)
and [Apple's web-push requirements](https://developer.apple.com/documentation/usernotifications/sending-web-push-notifications-in-web-apps-and-browsers).

## Enable notifications

1. Open the installed app, or a supported desktop browser, and connect to boblm.
2. Click or tap the bell above the project list.
3. Allow notifications when the browser or operating system asks.

A crossed-out bell means notifications are off. A bell without the slash means
this browser or app is subscribed. Enable them separately on each device and in
each browser or installed app you use.

If the bell is missing, check that you're using HTTPS and a supported browser.
On iPhone, launch from the Home Screen. The server also needs push configured;
see [the push configuration details](adr/0034-push-notifications.md).

New questions from the agent’s `ask` tool and credential requests send notifications.
Edits, failures and completed turns do not. To test delivery, switch to another app
before asking an agent to create a question through `ask`.
BobLM suppresses notifications for a chat you're already viewing in a focused
app window.

If the bell is on but alerts don't appear, check the app's notification permission
in the operating system and whether Focus is silencing it. On Mac, look under
System Settings > Notifications for the installed app's name, as described in
[Apple's notification instructions](https://support.apple.com/en-gb/104996).

## Keeping the Claude CLI current

BobLM turns off Claude's own updater, so a nightly systemd timer updates it and checks the new
version with one real turn, rolling back on failure (ADR 0052). Install it with
`bin/install-claude-update-timer` and turn it off with `bin/install-claude-update-timer --remove`.
It is silent: read outcomes with `journalctl --user -u boblm-claude-update` or
`~/.local/state/boblm/claude-update.json`. `bin/claude-update-check --dry-run` shows what it would do.

## Database backups

Run `bin/backup-db --dry-run` to inspect the plan, then `bin/install-backup-timer` to enable
nightly verified database backups. The newest three managed backups survive, including labeled
pre-migration backups. Read outcomes with `journalctl --user -u boblm-db-backup` and
`~/.local/state/boblm/db-backup.json`. Backups contain chat history, including chats deleted
since the copy was taken. Turn the timer off with `bin/install-backup-timer --remove`.
See [ADR 0053](adr/0053-database-backups.md) for configuration and restoration.
