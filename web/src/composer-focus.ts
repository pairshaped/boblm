/** Put background chat input in the composer without taking focus from controls. */
export function listenForComposerFocus(document: Document): () => void {
  const interactive =
    'input, textarea, select, button, a, summary, [contenteditable], [role="button"], [role="dialog"]';
  let chatSelected = false;
  const eligible = (target: EventTarget | null): boolean => {
    if (!(target instanceof document.defaultView!.Element)) return false;
    if (target.closest(interactive)) return false;
    return (
      !!target.closest(".chat") || (target === document.body && chatSelected)
    );
  };
  const focus = (): void => {
    const composer = document.querySelector<HTMLTextAreaElement>("#composer");
    if (composer && !composer.disabled && !composer.readOnly)
      composer.focus({ preventScroll: true });
  };
  const click = (event: MouseEvent): void => {
    const target = event.target;
    chatSelected =
      target instanceof document.defaultView!.Element &&
      !!target.closest(".chat");
    if (eligible(target) && document.getSelection()?.isCollapsed !== false)
      focus();
  };
  const type = (event: KeyboardEvent): void => {
    if (
      event.key === "Escape" &&
      !event.defaultPrevented &&
      !event.repeat &&
      !event.isComposing &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !document.querySelector('[role="dialog"], dialog[open]') &&
      event.target instanceof document.defaultView!.Element &&
      (event.target.closest(".chat, .compose") ||
        (event.target === document.body && chatSelected)) &&
      !event.target.closest("select")
    ) {
      const stop = document.querySelector<HTMLButtonElement>(
        ".compose button.stop",
      );
      if (stop && !stop.disabled) {
        event.preventDefault();
        stop.click();
      }
      return;
    }
    if (
      !event.defaultPrevented &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !event.isComposing &&
      event.key.length === 1 &&
      eligible(event.target)
    )
      focus();
  };
  document.addEventListener("click", click);
  document.addEventListener("keydown", type);
  return () => {
    document.removeEventListener("click", click);
    document.removeEventListener("keydown", type);
  };
}
