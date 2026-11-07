import type { ChatRow, ToolDetail } from "./timeline.ts";

export type VisibleRow = Omit<ChatRow, "seq" | "revision"> & {
  toolGroupLabel?: string;
  sourceKeys?: ReadonlyArray<string>;
};

const standalone = new Set([
  "agent",
  "task",
  "skill",
  "compactionsummary",
  "todowrite",
  "update_plan",
  "exitplanmode",
  "exit_plan_mode",
  "askuserquestion",
  "request_user_input",
  "request_user_input_async",
  "request_sops_secret",
  "spawn_peer",
  "ping_peer",
  "spawn_agent",
  "send_input",
  "send_message",
  "sendmessage",
  "resume_agent",
  "followup_task",
  "wait_agent",
  "close_agent",
  "interrupt_agent",
  "list_agents",
  "teamcreate",
  "teamdelete",
  "codexpermission",
  "collabagenttoolcall",
]);

function routine(tool: ToolDetail): boolean {
  const name = tool.name.toLowerCase().split(/__|\./).at(-1)!;
  return !tool.error && !standalone.has(name);
}

/** Group presentation only; persisted rows and pagination boundaries stay intact. */
export function groupToolRows(rows: ReadonlyArray<VisibleRow>): VisibleRow[] {
  const visible: VisibleRow[] = [];
  let run: VisibleRow[] = [];
  const flush = () => {
    if (!run.length) return;
    const tools = run.flatMap((row) =>
      row.tools!.map((tool) => ({ ...tool, key: `${row.key}:${tool.key}` })),
    );
    if (tools.length === 1 && tools[0]!.kind === "result") visible.push(...run);
    else {
      const calls = tools.filter((tool) => tool.kind === "call");
      const names = [
        ...new Set((calls.length ? calls : tools).map((tool) => tool.name)),
      ];
      const count = new Set(
        (calls.length ? calls : tools).map((tool) => tool.callId ?? tool.key),
      ).size;
      visible.push({
        ...run[0]!,
        role: "tool",
        tools,
        sourceKeys: run.map((row) => row.key),
        toolGroupLabel: `${count} tool ${calls.length ? (count === 1 ? "call" : "calls") : count === 1 ? "result" : "results"} · ${names.join(", ")}`,
      });
    }
    run = [];
  };
  for (const row of rows) {
    if (
      !row.text &&
      row.tools?.length &&
      !row.attachments?.length &&
      row.tools.every(routine)
    ) {
      run.push(row);
    } else {
      flush();
      visible.push(row);
    }
  }
  flush();
  return visible;
}
