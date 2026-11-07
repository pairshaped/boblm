# 0018: Where tunable values live

BobLM has no settings screens (ADR 0009). Anyone who wants different behavior
edits code and rebuilds; an agent can do that as easily as a person. The values
a fork is most likely to change are kept in two small, isolated places so those
edits rarely collide with upstream changes.

`crates/tuning` holds timeouts, size and count limits, the sweep age, the default
port and the Claude launch options (binary, permission mode, launch model,
disallowed tools, appended system prompt, extra arguments). It has no dependencies,
so changing a value rebuilds quickly (a few seconds across the workspace). Each
constant is one documented line, in alphabetical order, so a merge conflicts only
when both sides changed the same constant. Code elsewhere never repeats a tunable
literal. Limits the browser contract depends on are marked as contract values; the
protocol generator emits them to the browser (`limits` in the generated contract, and the body
limit inside the schema the decoder reads),
and `bin/check` fails until the generated files are refreshed, so the two sides
cannot drift.

`web/src/theme.css` holds the colors and base font sizes as CSS variables.
`web/src/style.css` is layout and uses the variables only. The build concatenates
the two into one stylesheet.

The model catalog (`MODELS`: model, agent, efforts) is a tunable value like the rest
and lives in the tuning crate (ADR 0023); the server serves it to the browser.
Instance setup (token, database path, port) stays
in the JSON config that `boblm init` creates (ADR 0016); it holds a secret and is
per installation, not per fork.

Values that protect the owner are not tunables and stay next to the code that
enforces them: the loopback-only bind, the private file modes, the set of image
types served inline, the sanitization of stored file names, and the retry count
that the database constrains.

Other ADRs cite these values by constant name with the shipped default in
parentheses; the tuning crate is authoritative, so a fork changes the value in
one place and the prose still names where to look.

A change takes effect after a rebuild and a restart. A web-only restart keeps supervised
runners and providers alive. Driver or runner changes require replacing the
affected runners when safe (ADR 0040).
