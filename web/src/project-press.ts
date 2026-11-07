import { limits } from "./generated/contract.ts";

/** Recognize a hold without turning a click, scroll, or project drag into a rename. */
export function listenForProjectRename(
  document: Document,
  rename: (id: string) => void,
): () => void {
  let pending: {
    button: HTMLElement;
    pointer: number;
    x: number;
    y: number;
    timer: ReturnType<typeof setTimeout>;
  } | null = null;
  let held: HTMLElement | null = null;
  const buttonAt = (target: EventTarget | null): HTMLElement | null =>
    target instanceof document.defaultView!.Element
      ? target.closest<HTMLElement>("[data-toggle-project]")
      : null;
  const cancel = () => {
    if (pending) clearTimeout(pending.timer);
    pending = null;
  };
  const down = (event: PointerEvent) => {
    cancel();
    held = null;
    const button = buttonAt(event.target);
    if (!button || event.button !== 0 || !event.isPrimary) return;
    pending = {
      button,
      pointer: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      timer: setTimeout(() => {
        pending = null;
        if (!button.isConnected) return;
        held = button;
        rename(button.dataset.toggleProject!);
      }, limits.longPressMs),
    };
  };
  const move = (event: PointerEvent) => {
    if (
      pending &&
      event.pointerId === pending.pointer &&
      Math.hypot(event.clientX - pending.x, event.clientY - pending.y) >
        limits.longPressMovePx
    )
      cancel();
  };
  const click = (event: MouseEvent) => {
    if (held && buttonAt(event.target) === held) {
      event.preventDefault();
      event.stopImmediatePropagation();
      held = null;
    }
  };
  const key = (event: KeyboardEvent) => {
    const button = buttonAt(event.target);
    if (button && event.key === "F2" && !event.repeat) {
      event.preventDefault();
      rename(button.dataset.toggleProject!);
    }
  };
  const context = (event: MouseEvent) => {
    if (buttonAt(event.target) && (pending || held)) event.preventDefault();
  };
  document.addEventListener("pointerdown", down);
  document.addEventListener("pointermove", move);
  document.addEventListener("pointerup", cancel);
  document.addEventListener("pointercancel", cancel);
  document.addEventListener("dragstart", cancel);
  document.addEventListener("scroll", cancel, true);
  document.addEventListener("click", click, true);
  document.addEventListener("keydown", key);
  document.addEventListener("contextmenu", context);
  document.defaultView?.addEventListener("blur", cancel);
  return () => {
    cancel();
    document.removeEventListener("pointerdown", down);
    document.removeEventListener("pointermove", move);
    document.removeEventListener("pointerup", cancel);
    document.removeEventListener("pointercancel", cancel);
    document.removeEventListener("dragstart", cancel);
    document.removeEventListener("scroll", cancel, true);
    document.removeEventListener("click", click, true);
    document.removeEventListener("keydown", key);
    document.removeEventListener("contextmenu", context);
    document.defaultView?.removeEventListener("blur", cancel);
  };
}
