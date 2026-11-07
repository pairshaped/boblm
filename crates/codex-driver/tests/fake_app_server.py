"""A fake `codex app-server` that speaks the message shapes observed live (docs/live-codex-protocol.md)."""
import json
import os
import sys
import threading
import time
import uuid

for feature in ("multi_agent", "multi_agent_v2"):
    assert "features.%s=false" % feature in sys.argv, sys.argv

lock = threading.Lock()
# What the real Codex accepts, recorded by bin/refresh-codex-schema. A request that sends a field
# the schema lacks, or leaves out a required one, is refused here so the driver tests fail.
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "schema", "surface.json")) as f:
    SURFACE = json.load(f)["requests"]
state = {"thread": None, "turn": None, "queue": []}
requests_seen = []
if "FAKE_LOG" in os.environ:
    with open(os.environ["FAKE_LOG"], "a") as log:
        log.write("codex_home=%s\n" % os.environ.get("CODEX_HOME", ""))
        log.write("openai_key_present=%s\n" % bool(os.environ.get("OPENAI_API_KEY")))


def out(message):
    with lock:
        sys.stdout.write(json.dumps(message) + "\n")
        sys.stdout.flush()


def serial(line):
    path = os.environ.get("FAKE_SERIAL_LOG")
    if path:
        with open(path, "a") as log:
            log.write(line + "\n")


def reply(rid, result):
    out({"id": rid, "result": result})


def fail(rid, code, message):
    out({"id": rid, "error": {"code": code, "message": message}})


def notify(method, params):
    out({"method": method, "params": params})


def user_item(text, client_id):
    return {"type": "userMessage", "id": str(uuid.uuid4()), "clientId": client_id, "content": [{"type": "text", "text": text}]}


def finish_turn(status="completed", error=None):
    turn = state["turn"]
    if turn is None:
        return
    state["turn"] = None
    notify("thread/status/changed", {"threadId": state["thread"], "status": {"type": "idle"}})
    done = {"id": turn, "status": status, "items": []}
    if error:
        done["error"] = {"message": error}
    notify("turn/completed", {"threadId": state["thread"], "turn": done})


def agent_reply(text):
    item = str(uuid.uuid4())
    notify("item/started", {"threadId": state["thread"], "turnId": state["turn"], "item": {"type": "agentMessage", "id": item, "text": "", "phase": "final_answer"}})
    for word in text.split(" "):
        notify("item/agentMessage/delta", {"threadId": state["thread"], "turnId": state["turn"], "itemId": item, "delta": word + " "})
    notify("item/completed", {"threadId": state["thread"], "turnId": state["turn"], "item": {"type": "agentMessage", "id": item, "text": text, "phase": "final_answer"}})


def run_turn(text, client_id, params):
    def work():
        if text == "capacity":
            notify("error", {"threadId": state["thread"], "turnId": state["turn"], "error": {"message": "model is at capacity", "codexErrorInfo": "serverOverloaded"}, "willRetry": True})
            return
        if text == "diagnostics":
            notify("error", {"threadId": state["thread"], "turnId": state["turn"], "error": {"message": "provider connection failed"}, "willRetry": False})
            sys.stderr.write("oversized-secret-" + "x" * int(os.environ.get("FAKE_DIAGNOSTIC_LIMIT", "0")) + "\n")
            sys.stderr.write("upstream unavailable fake-capability\n")
            sys.stderr.write("final diagnostic without newline")
            sys.stderr.flush()
            with lock:
                sys.stdout.write("not JSON\n")
                sys.stdout.flush()
            time.sleep(0.1)
            os._exit(7)
        notify("item/started", {"threadId": state["thread"], "turnId": state["turn"], "item": user_item(text, client_id)})
        notify("item/completed", {"threadId": state["thread"], "turnId": state["turn"], "item": user_item(text, client_id)})
        if text == "slow":
            deadline = time.time() + 20
            while state["turn"] is not None and time.time() < deadline:
                time.sleep(0.02)
            return
        if text == "tool":
            cmd = {"type": "commandExecution", "id": "exec-1", "command": "echo hi", "status": "inProgress", "aggregatedOutput": None}
            notify("item/started", {"threadId": state["thread"], "turnId": state["turn"], "item": cmd})
            notify("item/completed", {"threadId": state["thread"], "turnId": state["turn"], "item": dict(cmd, status="completed", aggregatedOutput="hi\n", exitCode=0)})
        if text == "ask":
            out({"id": 900, "method": "item/tool/requestUserInput", "params": {"threadId": state["thread"], "turnId": state["turn"], "itemId": "q", "questions": []}})
            time.sleep(0.3)
        if text == "fail":
            finish_turn("failed", "model overloaded")
            return
        settings = "model=%s effort=%s" % (params.get("model"), params.get("effort"))
        agent_reply("echo: %s [%s]" % (text, settings))
        finish_turn("completed")
    threading.Thread(target=work, daemon=True).start()


def handle(message):
    method, rid, params = message.get("method"), message.get("id"), message.get("params") or {}
    requests_seen.append(method)
    if method is None:  # a response to a server request
        if message.get("id") == 900:
            with open(os.environ.get("FAKE_LOG", "/dev/null"), "a") as log:
                log.write("answered-900 error=%s\n" % bool(message.get("error")))
        return
    spec = SURFACE.get(method)
    if spec is not None:
        unknown = sorted(set(params) - set(spec["properties"]))
        missing = sorted(set(spec["required"]) - set(params))
        if unknown or missing:
            fail(rid, -32602, "schema mismatch in %s: unknown %s, missing %s" % (method, unknown, missing))
            return
    if method == "initialize":
        if os.environ.get("FAKE_INIT_DELAY_MS"):
            time.sleep(int(os.environ["FAKE_INIT_DELAY_MS"]) / 1000)
        reply(rid, {"userAgent": "fake/0", "codexHome": "/fake"})
        serial("ready %d" % os.getpid())
    elif method == "initialized":
        pass
    elif method in ("thread/loaded/list", "thread/backgroundTerminals/list"):
        try:
            with open(os.environ["FAKE_JOBS"]) as fixture:
                jobs = json.load(fixture)
        except (KeyError, OSError, ValueError):
            return fail(rid, -32601, "inventory unavailable")
        if method == "thread/loaded/list":
            if params.get("cursor") is None:
                reply(rid, {"data": [state["thread"]], "nextCursor":"children"})
            else:
                reply(rid, {"data": ["child-thread"], "nextCursor":None})
        else:
            owner = "parent" if params["threadId"] == state["thread"] else "child"
            items = jobs.get(owner, [])
            offset = int(params.get("cursor") or 0)
            reply(rid, {"data": items[offset:offset+1], "nextCursor":str(offset+1) if offset+1 < len(items) else None})
    elif method == "account/rateLimits/read":
        reply(rid, {"rateLimits":{"limitId":"codex","credits":{"hasCredits":True,"unlimited":False,"balance":"40000"},"primary":{"usedPercent":80,"windowDurationMins":300},"secondary":{"usedPercent":37,"windowDurationMins":10080,"resetsAt":1893456000}}})
        notify("thread/tokenUsage/updated", {"threadId":"child-thread","tokenUsage":{"last":{"inputTokens":999,"cachedInputTokens":0,"cacheWriteInputTokens":0,"outputTokens":1,"reasoningOutputTokens":0,"totalTokens":999},"modelContextWindow":1000}})
        notify("thread/tokenUsage/updated", {"threadId":state["thread"],"tokenUsage":{"last":{"inputTokens":80000,"cachedInputTokens":4000,"cacheWriteInputTokens":0,"outputTokens":100,"reasoningOutputTokens":20,"totalTokens":84000},"modelContextWindow":200000}})
    elif method in ("thread/start", "thread/resume"):
        if os.environ.get("FAKE_EXPECT_MCP"):
            cfg = (params.get("config") or {}).get("mcp_servers", {}).get("boblm", {})
            secret = "fake-capability"
            problems = []
            if cfg.get("url") != os.environ["FAKE_EXPECT_MCP"]:
                problems.append("url %r" % cfg.get("url"))
            if cfg.get("bearer_token_env_var") != "BOBLM_MCP_TOKEN":
                problems.append("token variable name %r" % cfg.get("bearer_token_env_var"))
            if os.environ.get("BOBLM_MCP_TOKEN") != secret:
                problems.append("token missing from the environment")
            if any(secret in argument for argument in sys.argv):
                problems.append("token in the arguments")
            if secret in json.dumps(params):
                problems.append("token in the request")
            with open(os.environ.get("FAKE_LOG", "/dev/null"), "a") as log:
                log.write("mcp-ok\n" if not problems else "mcp-bad: %s\n" % "; ".join(problems))
        assert params.get("approvalPolicy") == "never" and params.get("sandbox") == "danger-full-access", params
        assert (params.get("config") or {}).get("features") == {"multi_agent": False, "multi_agent_v2": False}, params
        state["thread"] = params.get("threadId") if method == "thread/resume" else "thread-" + str(uuid.uuid4())[:8]
        if method == "thread/resume":
            assert params.get("excludeTurns") is True, params
        if method == "thread/resume" and os.environ.get("FAKE_RESUME_FAILS"):
            return fail(rid, -32600, "no rollout found for thread id %s" % params.get("threadId"))
        reply(rid, {"thread": {"id": state["thread"]}})
        notify("thread/started", {"thread": {"id": state["thread"]}})
    elif method == "thread/turns/list":
        if os.environ.get("FAKE_TURN_READ_FAILS"):
            return fail(rid, -32603, "turn inventory unavailable")
        if os.environ.get("FAKE_TURN_READ_INVALID"):
            return reply(rid, {"data": [{"status": "inProgress"}]})
        if os.environ.get("FAKE_TURN_ENDS_BEFORE_READ"):
            state["turn"] = None
        turn = state["turn"]
        reply(rid, {"data": [{"id": turn, "status": "inProgress", "items": []}] if turn else []})
        if os.environ.get("FAKE_TURN_CHANGES_AGAIN"):
            state["turn"] = "turn-changed-again"
    elif method == "turn/start":
        if params.get("model") == "bad-model":
            return fail(rid, -32600, "unknown model bad-model")
        state["turn"] = "turn-" + str(uuid.uuid4())[:8]
        reply(rid, {"turn": {"id": state["turn"]}})
        notify("thread/status/changed", {"threadId": state["thread"], "status": {"type": "active", "activeFlags": []}})
        notify("turn/started", {"threadId": state["thread"], "turn": {"id": state["turn"]}})
        text = params["input"][0]["text"]
        if text == "child-turn":
            notify("turn/started", {"threadId": "child-thread", "turn": {"id": "child-turn"}})
            notify("turn/completed", {"threadId": "child-thread", "turn": {"id": "child-turn", "status": "completed"}})
            notify("item/started", {"threadId": state["thread"], "turnId": state["turn"], "item": user_item(text, params.get("clientUserMessageId"))})
            return
        if text == "replaced-turn":
            state["turn"] = "turn-current"
            return
        if text == "ghost":
            # The turn is gone on Codex's side but the driver was never told: a later steer is stale.
            state["turn"] = None
            return
        run_turn(text, params.get("clientUserMessageId"), params)
    elif method == "turn/steer":
        if state["turn"] is None:
            return fail(rid, -32600, "no active turn to steer")
        if params.get("expectedTurnId") != state["turn"]:
            return fail(rid, -32600, "expected active turn id `%s` but found `%s`" % (state["turn"], params.get("expectedTurnId")))
        reply(rid, {"turnId": state["turn"]})
        text = params["input"][0]["text"]
        item = user_item(text, params.get("clientUserMessageId"))
        notify("item/started", {"threadId": state["thread"], "turnId": state["turn"], "item": item})
        notify("item/completed", {"threadId": state["thread"], "turnId": state["turn"], "item": item})
        agent_reply("steered: %s" % text)
        finish_turn("completed")
    elif method == "turn/interrupt":
        reply(rid, {})
        finish_turn("interrupted")
    elif method == "thread/compact/start":
        reply(rid, {})
        if os.environ.get("FAKE_COMPACT_SILENT"):
            os._exit(0)  # accepted the request, then died without compacting anything
        state["turn"] = "turn-compact"
        notify("turn/started", {"threadId": state["thread"], "turn": {"id": state["turn"]}})
        notify("item/started", {"threadId": state["thread"], "turnId": state["turn"], "item": {"type": "contextCompaction", "id": "c1"}})
        notify("item/completed", {"threadId": state["thread"], "turnId": state["turn"], "item": {"type": "contextCompaction", "id": "c1"}})
        finish_turn("completed")
    else:
        if rid is not None:
            fail(rid, -32601, "method not found: %s" % method)


if os.environ.get("FAKE_EXIT_AT_START"):
    sys.exit(3)
serial("start %d" % os.getpid())
for line in sys.stdin:
    line = line.strip()
    if line:
        handle(json.loads(line))
