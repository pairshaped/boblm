import {
  emptySubscription,
  type ProgramSubscription,
} from "@pairshaped/hypertea/program";
import type { Model, Msg } from "./app.ts";
import { postCommand } from "./http.ts";
import { limits, parseSubagentHistoryPage } from "./generated/contract.ts";
import {
  attachScroll,
  restoreHistoryAnchor,
  type HistoryAnchor,
} from "./scroll.ts";
import type { Timeline } from "./timeline.ts";

export type SubagentConversation = {
  key: string;
  title: string;
  owner: string;
  /** True for a boblm child session, which uses chat history and its live stream. */
  boblm: boolean;
  generation: number;
  before?: number | undefined;
  hasNewer: boolean;
  timeline: Timeline;
  cursor: number;
  hasOlder: boolean;
  loading: boolean;
  following: boolean;
  error: string;
};
/** The drawer owns a separate stream and scroll anchor; it never changes the parent chat cursor. */
export function subagentConversationSubscription(
  model: Model,
): ProgramSubscription<Msg> {
  const view = model.subagentConversation;
  if (
    !model.token ||
    model.connectionPending ||
    model.hiddenSince !== null ||
    !model.drawerOpen ||
    model.drawerTab !== "agents" ||
    !view
  )
    return emptySubscription<Msg>("slot:subagent-conversation");
  if (view.boblm) {
    return {
      key: `subagent-scroll:${view.owner}:${view.key}:${view.generation}`,
      subscribe: (dispatch) => {
        const state: { anchor: HistoryAnchor | null } = { anchor: null };
        let frame = 0;
        let detach = () => {};
        let tries = 0;
        const attach = () => {
          const element =
            document.querySelector<HTMLElement>("#subagent-messages");
          if (element && element.dataset.sessionId === view.key) {
            detach = attachScroll(
              element,
              view.key,
              (msg) => {
                if (msg.type === "following")
                  dispatch({ type: "subagentFollowing", value: msg.value });
                if (msg.type === "history" && msg.direction === "older")
                  dispatch({ type: "subagentPage", latest: false });
              },
              state,
            );
          } else if (tries++ < 300) frame = requestAnimationFrame(attach);
        };
        attach();
        return () => {
          cancelAnimationFrame(frame);
          detach();
        };
      },
    };
  }
  return {
    key: `subagent:${view.owner}:${view.key}:${view.generation}`,
    subscribe: (dispatch) => {
      const controller = new AbortController();
      const state: { anchor: HistoryAnchor | null } = { anchor: null };
      let stopped = false;
      let timer: ReturnType<typeof setTimeout> | undefined;
      let frame = 0;
      let detach = () => {};
      let tries = 0;
      let firstRead = true;
      const attach = () => {
        const element =
          document.querySelector<HTMLElement>("#subagent-messages");
        if (element && element.dataset.sessionId === view.key) {
          detach = attachScroll(
            element,
            view.key,
            (msg) => {
              if (msg.type === "following")
                dispatch({ type: "subagentFollowing", value: msg.value });
              if (msg.type === "history" && msg.direction === "older")
                dispatch({
                  type: "subagentPage",
                  latest: false,
                });
            },
            state,
          );
        } else if (tries++ < 300) frame = requestAnimationFrame(attach);
      };
      const read = async () => {
        const messages =
          document.querySelector<HTMLElement>("#subagent-messages");
        if (messages && messages.dataset.sessionId === view.key) {
          const away =
            messages.scrollHeight - messages.scrollTop - messages.clientHeight >
              80 || view.before !== undefined;
          if (away) {
            const bounds = messages.getBoundingClientRect();
            const row = [
              ...messages.querySelectorAll<HTMLElement>("[data-row-key]"),
            ].find((row) => row.getBoundingClientRect().bottom > bounds.top);
            if (row?.dataset.rowKey)
              state.anchor = {
                sessionId: view.key,
                generation: view.generation,
                key: row.dataset.rowKey,
                offset: row.getBoundingClientRect().top - bounds.top,
              };
          }
        }
        const outcome = await postCommand(
          "/commands/subagentHistory",
          {
            sessionId: view.owner,
            agentKey: view.key,
            ...(view.before === undefined ? {} : { before: view.before }),
          },
          model.token,
          parseSubagentHistoryPage,
          (url, init) => fetch(url, { ...init, signal: controller.signal }),
        );
        if (stopped) return;
        const initial = firstRead;
        firstRead = false;
        dispatch({
          type: "subagentReply",
          generation: view.generation,
          owner: view.owner,
          key: view.key,
          outcome,
        });
        requestAnimationFrame(() =>
          requestAnimationFrame(() => {
            if (stopped) return;
            const element =
              document.querySelector<HTMLElement>("#subagent-messages");
            if (element?.dataset.sessionId === view.key) {
              if (
                initial &&
                view.loading &&
                view.before === undefined &&
                view.following
              )
                element.scrollTop = element.scrollHeight;
              else restoreHistoryAnchor(element, state);
              state.anchor = null;
              element.dispatchEvent(new Event("scroll"));
            }
          }),
        );
        if (view.before === undefined)
          timer = setTimeout(read, limits.subagentPollMs);
      };
      attach();
      void read();
      return () => {
        stopped = true;
        controller.abort();
        clearTimeout(timer);
        cancelAnimationFrame(frame);
        detach();
      };
    },
  };
}
