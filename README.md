# BobLM

A small, self-hosted web app for running coding agents from your desktop or phone.
It runs Claude Code, Codex and Pi (DeepSeek through DeepInfra) on one machine, keeps
chat history in SQLite, and streams their work to your browser.

## Why I built it

I built BobLM as a smaller alternative to HAPI, focused on the parts I
actually use. I wanted a responsive app without carrying around a pile of features
I don't need.

The backend is Rust. The frontend is TypeScript with Hypertea. Browser assets are
bundled into the server binary, including a single JavaScript bundle and stylesheet.
The goal is to keep the app fast and avoid unnecessary asset loading. Long chats
use a bounded rendering window instead of keeping the entire conversation in the
DOM. There are [profiling results](docs/performance/long-chat-2026-10-24.md), but
no head-to-head performance benchmark against HAPI.

Keeping the feature set small is part of that design. I'm building this for my
own workflow, and I intend to keep it that way.

## Configure it with code

I don't want a bunch of preferences and configuration screens. I find them
annoying. If you want different behavior, fork the project and change the code.

Defaults and limits live in [crates/tuning/src/lib.rs](crates/tuning/src/lib.rs).
Colors, font sizes and layout values live in [web/src/theme.css](web/src/theme.css).
Change those, rebuild, and run your version. Connection credentials and other
runtime details still live in a configuration file; that doesn't need a settings
screen in the app.

Fork and tweak is the intended customization model. Additional preferences screens
are outside the scope of this project.

## Forks and pull requests

You're welcome to fork BobLM and add the features you want. You're also welcome
to open a pull request, but opening one doesn't mean I'll merge it.

I'll merge features I care about that fit the scope of the app. If I don't want a
feature in my own workflow, I'll leave it out. Keeping it in your fork is fine.
For substantial changes, ask before investing time in a PR you need merged.

I intend to keep upstream changes small and infrequent enough that maintaining a
personal fork is practical. That's a maintenance goal, not a promise of no breaking
changes.

## Running and developing

See [the usage guide](docs/usage.md) for renaming and reordering projects and chats,
keyboard shortcuts, scroll controls, the right drawer, and installing the app
for notifications on Mac and iPhone.

See [the running guide](docs/running.md) for setup, authentication and browser
access. Marmot and Hypertea come from their public git repositories, pinned by
commit, so the repository builds on its own. A built server binary includes the
browser assets and doesn't need a source checkout to serve them.

For local Marmot development, you can add an untracked `.cargo/config.toml`:

```toml
[patch."https://github.com/pairshaped/marmot-rust"]
marmot = { path = "/path/to/marmot-rust" }
```

For local Hypertea development, build your checkout and run
`npm link /path/to/hypertea` in `web/`. The link only changes `web/node_modules`;
run `npm ci` in `web/` to return to the pinned commit.

Use `bin/dev-instance` when developing BobLM so testing a server restart doesn't
interrupt your real chats. Run `bin/check` to build the browser assets and verify
the workspace.

[CONTEXT.md](CONTEXT.md) defines the project vocabulary.
[The architecture decisions](docs/adr/) describe the implementation choices.
