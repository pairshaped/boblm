import {
  emptySubscription,
  type ProgramSubscription,
} from "@pairshaped/hypertea/program";
import type { Model, Msg } from "./app.ts";
import { limits } from "./generated/contract.ts";

/** Where the owner was reading before a page of older or newer history was added. */
export type HistoryAnchor = {
  sessionId: string;
  generation: number;
  key: string;
  offset: number;
};
/** History paging and the scroll handling share this bookkeeping. */
export const historyScroll: {
  anchor: HistoryAnchor | null;
  controller: AbortController | null;
} = { anchor: null, controller: null };

export function restoreHistoryAnchor(
  messages: HTMLElement,
  state: { anchor: HistoryAnchor | null } = historyScroll,
) {
  const anchor = state.anchor;
  if (!anchor || messages.dataset.sessionId !== anchor.sessionId) return;
  const row = [
    ...messages.querySelectorAll<HTMLElement>("[data-row-key]"),
  ].find(
    (el) =>
      el.dataset.rowKey === anchor.key ||
      (JSON.parse(el.dataset.rowKeys ?? "[]") as string[]).includes(anchor.key),
  );
  if (row)
    messages.scrollTop +=
      row.getBoundingClientRect().top -
      messages.getBoundingClientRect().top -
      anchor.offset;
}

/**
 * Follow the newest messages while the owner is at the bottom, and hold the place once they scroll
 * away. Returns what undoes it.
 */
export function attachScroll(
  messages: HTMLElement,
  active: string,
  dispatch: (msg: Msg) => void,
  state: { anchor: HistoryAnchor | null } = historyScroll,
): () => void {
  if (state.anchor?.sessionId !== active) state.anchor = null;
  let following = true;
  let intent = 0;
  let intentAt = 0;
  let lastPointerY: number | null = null;
  let lastTouchY: number | null = null;
  const page = () => {
    if (state.anchor || messages.dataset.historyLoading === "true") {
      intent = 0;
      return;
    }
    const direction =
      Date.now() - intentAt <= limits.historyScrollIntentMs ? intent : 0;
    const older =
      direction < 0 &&
      messages.scrollTop < limits.historyScrollThreshold &&
      messages.dataset.hasOlder === "true";
    const newer =
      direction > 0 &&
      messages.scrollHeight - messages.scrollTop - messages.clientHeight <
        limits.historyScrollThreshold &&
      messages.dataset.hasNewer === "true";
    if (!older && !newer) return;
    // One gesture can request one page. Anchor restoration and layout changes are not gestures.
    intent = 0;
    if (older) {
      following = false;
      dispatch({ type: "following", value: false });
    }
    dispatch({ type: "history", direction: older ? "older" : "newer" });
  };
  const gesture = (direction: number) => {
    intent = Math.sign(direction);
    intentAt = Date.now();
    page();
  };
  const wheel = (event: WheelEvent) => {
    if (Math.abs(event.deltaY) >= Math.abs(event.deltaX)) gesture(event.deltaY);
  };
  const key = (event: KeyboardEvent) => {
    if (
      (event.target as HTMLElement | null)?.closest?.(
        "button,input,textarea,select,a",
      )
    )
      return;
    if (
      ["ArrowUp", "PageUp", "Home"].includes(event.key) ||
      (event.key === " " && event.shiftKey)
    )
      gesture(-1);
    else if (["ArrowDown", "PageDown", "End", " "].includes(event.key))
      gesture(1);
  };
  const pointerDown = (event: PointerEvent) => {
    if (event.pointerType !== "touch") lastPointerY = event.clientY;
  };
  const pointerMove = (event: PointerEvent) => {
    if (lastPointerY === null) return;
    const movement = event.clientY - lastPointerY;
    lastPointerY = event.clientY;
    if (state.anchor || messages.dataset.historyLoading === "true") {
      intent = 0;
      return;
    }
    if (movement) {
      intent = Math.sign(movement);
      intentAt = Date.now();
    }
  };
  const pointerUp = () => {
    lastPointerY = null;
  };
  const touchStart = (event: TouchEvent) => {
    lastTouchY = event.touches[0]?.clientY ?? null;
  };
  const touchMove = (event: TouchEvent) => {
    const y = event.touches[0]?.clientY;
    if (y !== undefined && lastTouchY !== null) gesture(lastTouchY - y);
    lastTouchY = y ?? null;
  };
  const scroll = () => {
    if (state.anchor || messages.dataset.historyLoading === "true") {
      intent = 0;
      return;
    }
    following =
      messages.scrollHeight - messages.scrollTop - messages.clientHeight <=
        80 && messages.dataset.hasNewer !== "true";
    dispatch({ type: "following", value: following });
    if (following) dispatch({ type: "newMessages", visible: false });
    page();
  };
  messages.addEventListener("wheel", wheel, { passive: true });
  messages.addEventListener("keydown", key);
  messages.addEventListener("pointerdown", pointerDown);
  window.addEventListener("pointermove", pointerMove);
  window.addEventListener("pointerup", pointerUp);
  window.addEventListener("pointercancel", pointerUp);
  messages.addEventListener("touchstart", touchStart, { passive: true });
  messages.addEventListener("touchmove", touchMove, { passive: true });
  messages.addEventListener("scroll", scroll);
  // Layout changes do not fire a scroll event: the composer growing while typing or dictating, the
  // phone keyboard, the window resizing, an image finishing loading. Following means staying put at
  // the bottom through all of them.
  const pin = () => {
    if (state.anchor || messages.dataset.historyLoading === "true") return;
    if (following && messages.dataset.hasNewer !== "true")
      messages.scrollTop = messages.scrollHeight;
  };
  // The lock sign sits under the scrollbar and needs that bar's real width, which differs by system
  // and zoom. An overlay scrollbar takes no room, so the style's own width is used for it.
  const measureScrollbar = () => {
    const width = messages.offsetWidth - messages.clientWidth;
    if (width > 0)
      messages.parentElement?.style.setProperty(
        "--scrollbar-width",
        `${width}px`,
      );
    else messages.parentElement?.style.removeProperty("--scrollbar-width");
  };
  measureScrollbar();
  const resized =
    typeof ResizeObserver === "function"
      ? new ResizeObserver(() => {
          measureScrollbar();
          pin();
        })
      : null;
  resized?.observe(messages);
  messages.addEventListener("load", pin, true);
  const observer = new MutationObserver(() => {
    if (state.anchor) {
      restoreHistoryAnchor(messages, state);
      return;
    }
    if (messages.dataset.historyLoading === "true") return;
    if (following && messages.dataset.hasNewer !== "true")
      messages.scrollTop = messages.scrollHeight;
    else dispatch({ type: "newMessages", visible: true });
  });
  observer.observe(messages, {
    childList: true,
    subtree: true,
    characterData: true,
  });
  return () => {
    observer.disconnect();
    resized?.disconnect();
    messages.removeEventListener("load", pin, true);
    messages.removeEventListener("scroll", scroll);
    messages.removeEventListener("wheel", wheel);
    messages.removeEventListener("keydown", key);
    messages.removeEventListener("pointerdown", pointerDown);
    window.removeEventListener("pointermove", pointerMove);
    window.removeEventListener("pointerup", pointerUp);
    window.removeEventListener("pointercancel", pointerUp);
    messages.removeEventListener("touchstart", touchStart);
    messages.removeEventListener("touchmove", touchMove);
  };
}

export function scrollSubscription(model: Model): ProgramSubscription<Msg> {
  if (!model.active) return emptySubscription<Msg>("slot:scroll");
  const active = model.active;
  return {
    key: `scroll:${active}`,
    subscribe: (dispatch) => {
      // A chat that is open from the first draw (one reopened at startup) can be subscribed to
      // before its message area is on the page, and the subscription is not retried. So look again
      // until it appears.
      let detach = () => {};
      let frame = 0;
      let tries = 0;
      const attach = () => {
        const messages = document.querySelector<HTMLElement>("#messages");
        if (messages) detach = attachScroll(messages, active, dispatch);
        else if (tries++ < 300) frame = requestAnimationFrame(attach);
      };
      attach();
      return () => {
        cancelAnimationFrame(frame);
        detach();
      };
    },
  };
}
