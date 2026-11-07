# BobLM

A chat server and browser app for running coding agents (Claude, Codex) from one place. This file is the project glossary; design decisions live in `docs/adr/`.

## Language

**Active chat**:
A chat that is not archived. It says nothing about whether the agent is working.
_Avoid_: Open chat, live chat, running chat

**Archived chat**:
A chat the owner has set aside. It keeps its history and can be reopened, and it is the only kind of chat that can be deleted.
_Avoid_: Closed chat, inactive chat

**Mid-turn**:
A chat whose agent is working on a turn right now.
_Avoid_: Active, running, busy, thinking
