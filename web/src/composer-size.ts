/** Size a textarea to its content. The CSS max-height caps the growth, and scrolling takes over past it. */
export function fitComposer(textarea: HTMLTextAreaElement): void {
  const messages =
    textarea.ownerDocument.querySelector<HTMLElement>("#messages");
  const scrolledTo = messages?.scrollTop;
  // Measure from an unconstrained height so the box shrinks when text is deleted or sent.
  textarea.style.height = "auto";
  const borders = textarea.offsetHeight - textarea.clientHeight;
  textarea.style.height = `${textarea.scrollHeight + borders}px`;
  // For that moment the box was short, so the message list above it was taller, and the browser lowered
  // its scroll position to fit. It does not rise again by itself, and the chat would stop following
  // the newest messages. Put it back.
  if (messages && scrolledTo !== undefined && messages.scrollTop !== scrolledTo)
    messages.scrollTop = scrolledTo;
}
