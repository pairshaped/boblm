import {
  emptySubscription,
  type ProgramSubscription,
} from "@pairshaped/hypertea/program";
import type { Model, Msg } from "./app.ts";
import { limits } from "./generated/contract.ts";

/** Resume an idle chat through its latest snapshot, never through missed live events. */
export function historyResumeSubscription(
  model: Model,
): ProgramSubscription<Msg> {
  if (!model.token || model.connectionPending)
    return emptySubscription<Msg>("slot:visibility");
  const id = model.active;
  return {
    key: `visible:${id ?? ""}`,
    subscribe: (dispatch) => {
      let stopped = false;
      let lastMoved = Date.now();
      let lastTop: number | null = null;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const schedule = () => {
        clearTimeout(timer);
        if (id && document.visibilityState === "visible")
          timer = setTimeout(
            back,
            Math.max(
              0,
              limits.historyResumeLatestMs - (Date.now() - lastMoved),
            ),
          );
      };
      const back = () => {
        if (stopped) return;
        const now = Date.now();
        const hidden = document.visibilityState === "hidden";
        const idle = !!id && now - lastMoved >= limits.historyResumeLatestMs;
        if (!hidden && idle) lastMoved = now;
        dispatch({ type: "visibility", hidden, now, idle });
        schedule();
      };
      const scroll = (event: Event) => {
        const element = event.target;
        if (
          !(element instanceof HTMLElement) ||
          element.id !== "messages" ||
          element.dataset.sessionId !== id
        )
          return;
        if (lastTop === element.scrollTop) return;
        lastTop = element.scrollTop;
        lastMoved = Date.now();
        schedule();
      };
      document.addEventListener("scroll", scroll, true);
      document.addEventListener("visibilitychange", back);
      window.addEventListener("focus", back);
      window.addEventListener("pageshow", back);
      // Subscription setup must finish before a visibility update can resubscribe.
      if (document.visibilityState === "hidden") queueMicrotask(back);
      else schedule();
      return () => {
        stopped = true;
        clearTimeout(timer);
        document.removeEventListener("scroll", scroll, true);
        document.removeEventListener("visibilitychange", back);
        window.removeEventListener("focus", back);
        window.removeEventListener("pageshow", back);
      };
    },
  };
}
