import json
import os
import subprocess
import sys
import time


def emit(value):
    print(json.dumps(value), flush=True)


settings = json.loads(sys.argv[sys.argv.index("--settings") + 1])
hook = settings["hooks"]["PreToolUse"]
assert len(hook) == 1 and hook[0]["matcher"] == "Bash"
assert hook[0]["hooks"][0]["type"] == "command"
assert hook[0]["hooks"][0]["command"].endswith("' hook-background-bash")
assert "--include-partial-messages" in sys.argv
assert sys.argv[sys.argv.index("--permission-prompts") + 1] == "none"
assert set(sys.argv[sys.argv.index("--disallowedTools") + 1].split(",")) == {"Agent", "Task", "AskUserQuestion", "EnterPlanMode", "ExitPlanMode"}
assert sys.argv[sys.argv.index("--permission-mode") + 1] == "bypassPermissions"
if "EXPECT_RESUME" in os.environ:
    assert sys.argv[sys.argv.index("--resume") + 1] == os.environ["EXPECT_RESUME"]

if "EXPECT_MCP_URL" in os.environ:
    assert "--strict-mcp-config" in sys.argv
    assert "test-session-capability" not in " ".join(sys.argv), "the capability must not be on the command line"
    config_path = sys.argv[sys.argv.index("--mcp-config") + 1]
    assert os.stat(config_path).st_mode & 0o777 == 0o600
    assert os.stat(os.path.dirname(config_path)).st_mode & 0o777 == 0o700
    with open(config_path) as config_file:
        mcp = json.load(config_file)
    assert mcp == {"mcpServers": {"boblm": {"type": "http", "url": os.environ["EXPECT_MCP_URL"], "headers": {"Authorization": "Bearer test-session-capability"}}}}
emit({"type": "system", "subtype": "init", "session_id": "fake-session", "args": sys.argv[1:]})
active = False
queued_steer = None
model = "sonnet"
effort = None
for line in sys.stdin:
    message = json.loads(line)
    if message["type"] == "control_request":
        if os.environ.get("FAKE_INTERRUPT") == "hang":
            continue
        if os.environ.get("FAKE_INTERRUPT") == "exit":
            sys.exit(7)
        subtype = message["request"]["subtype"]
        if subtype == "set_model":
            model = message["request"].get("model") or "default"
        if subtype == "apply_flag_settings":
            effort = message["request"]["settings"].get("effortLevel")
        response = {"effective": {"model": model, "effortLevel": effort}}
        if subtype == "get_context_usage":
            assert message["request"]["detail"] == "summary", "meter must not request full token-count calls"
            if os.environ.get("FAKE_USAGE") == "ignore": continue
            response = {"totalTokens":84000,"maxTokens":200000,"percentage":42}
        elif subtype == "get_usage":
            assert message["request"]["skip_behaviors"] is True
            if os.environ.get("FAKE_USAGE") == "ignore": continue
            response = {"rate_limits_available":True,"rate_limits":{"five_hour":{"utilization":80,"resets_at":"2030-01-01T00:00:00Z"},"seven_day":{"utilization":37,"resets_at":"2030-01-01T00:00:00Z"}}}
        emit({"type": "control_response", "response": {"subtype": "success", "request_id": message["request_id"], "response": response}})
        if active and subtype == "interrupt":
            emit({"type": "result", "subtype": "interrupted", "is_error": False})
            active = False
        continue
    mode = message["message"]["content"]
    uuid = message["uuid"]
    active = True
    if mode.startswith("runner-"):
        emit({"type": "assistant", "message": {"received_uuid": uuid}})
        if mode == "runner-queued-steer":
            queued_steer = uuid
            emit({"type": "command_lifecycle", "command_uuid": uuid, "state": "queued"})
            continue
        if mode == "runner-unacknowledged-steer":
            continue
        emit({"type": "command_lifecycle", "command_uuid": uuid, "state": "started"})
        if mode == "runner-tool":
            emit({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "long-tool", "name": "Bash", "input": {}}]}})
        elif mode == "runner-release-steer":
            emit({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "long-tool", "content": "done"}]}})
            emit({"type": "result", "subtype": "success", "is_error": False})
            emit({"type": "command_lifecycle", "command_uuid": queued_steer, "state": "started"})
        elif mode == "runner-self-woken-job":
            emit({"type": "system", "subtype": "background_tasks_changed", "tasks": [{"task_id": "own-job", "task_type": "local_bash", "description": "long command"}]})
            emit({"type": "result", "subtype": "success", "is_error": False})
            emit({"type": "system", "subtype": "task_notification", "task_id": "own-job", "status": "completed"})
            emit({"type": "system", "subtype": "init", "session_id": "fake-session"})
            emit({"type": "system", "subtype": "background_tasks_changed", "tasks": []})
            emit({"type": "result", "subtype": "success", "is_error": False})
        continue
    if mode in ["api-billing-error", "api-result-error"]:
        emit({"type": "command_lifecycle", "command_uuid": uuid, "state": "started"})
        if mode == "api-billing-error":
            emit({"type": "assistant", "error": "billing_error", "message": {"id": "billing-error", "model": "haiku", "content": [{"type": "text", "text": "Credit balance is too low"}]}})
        emit({"type": "result", "subtype": "error_during_execution", "is_error": True, "errors": [] if mode == "api-billing-error" else ["Credit balance is too low"]})
        continue
    if mode == "block-input":
        emit({"type": "assistant", "message": {"blocked": True}})
        time.sleep(60)
        continue
    if mode == "exit":
        sys.exit(7)
    if mode == "prompt-suggestion":
        emit({"type": "prompt_suggestion", "suggestion": {"future": True}})
        emit({"type": "prompt_suggestion"})
        emit({"type": "future_suggestion_event"})
        emit({"type": "prompt_suggestion", "suggestion": "Explain the example", "uuid": "prediction", "session_id": "fake-session", "future": True})
        continue
    if mode == "flood-exit":
        for n in range(3000):
            emit({"type": "assistant", "message": {"n": n}})
        emit({"type": "assistant", "message": {"n": "last"}})
        sys.exit(0)
    if mode == "burst-exit":
        sys.stdout.write('{"type":"x"}\n' * 2000)
        sys.stdout.flush()
        sys.exit(0)
    if mode == "escape-chatter":
        # A descendant in its own process group keeps writing non-JSON to our stdout.
        subprocess.Popen(
            [sys.executable, "-c", "import sys\nline = 'noise\\n' * 1000\nwhile True:\n    sys.stdout.write(line); sys.stdout.flush()"],
            start_new_session=True,
        )
        time.sleep(0.2)
        sys.exit(0)
    if mode == "garbage":
        sys.stdout.buffer.write(b"\xff\xfe not utf8\n")
        sys.stdout.buffer.write(b"x" * (20 * 1024 * 1024) + b"\n")
        sys.stdout.buffer.flush()
        emit({"type": "command_lifecycle", "command_uuid": uuid, "state": "started"})
        continue
    if mode == "exit-with-descendant":
        subprocess.Popen(["sleep", "60"])
        sys.exit(7)
    if mode == "hang":
        continue
    if mode == "tree":
        child = subprocess.Popen(["sleep", "60"])
        emit({"type": "assistant", "message": {"parent": os.getpid(), "child": child.pid}})
        continue
    if mode == "wire":
        emit({"type": "assistant", "message": message})
    if mode == "unknown":
        print("malformed", flush=True)
        emit({"type": "future_event", "new_field": 1})
        emit({"type": "command_lifecycle", "command_uuid": uuid, "state": "future_state"})
        emit({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"text": "hello"}}})
    if mode == "result-then-completed":
        emit({"type": "result", "subtype": "success"})
    if mode in ("completed-only", "result-then-completed"):
        active = False
        emit({"type": "command_lifecycle", "command_uuid": uuid, "state": "completed"})
        continue
    if mode == "cutover":
        emit({"type": "result", "subtype": "success", "is_error": False})
        emit({"type": "command_lifecycle", "command_uuid": "original", "state": "cancelled"})
    emit({"type": "command_lifecycle", "command_uuid": uuid, "state": "queued"})
    state = mode if mode in ("cancelled", "discarded", "refused") else "started"
    emit({"type": "command_lifecycle", "command_uuid": uuid, "state": state, "extra": True})
