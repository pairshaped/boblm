/** Keep the composer as the app's only sequential keyboard focus target. */
export function limitTabStops(root: HTMLElement): () => void {
  const window = root.ownerDocument.defaultView!;
  // Scroll containers can be native tab stops even without a tabindex attribute.
  const targets =
    "a[href], button, input, textarea, select, summary, iframe, audio[controls], video[controls], [contenteditable], [tabindex], .sidebar, .sidebar > :not(header), .messages, .subagents-body, .table-scroll, pre, .directory-picker, .directory-list";
  const update = (node: Element): void => {
    if (!(node instanceof window.HTMLElement)) return;
    if (node.matches(targets)) {
      const index = node.id === "composer" ? 0 : -1;
      if (node.getAttribute("tabindex") !== String(index))
        node.tabIndex = index;
    }
    node.querySelectorAll(targets).forEach((element) => {
      const index = element.id === "composer" ? "0" : "-1";
      if (element.getAttribute("tabindex") !== index)
        element.setAttribute("tabindex", index);
    });
  };
  const observer = new window.MutationObserver((changes) => {
    for (const change of changes) {
      if (change.type === "attributes") {
        const target = change.target as HTMLElement;
        const index = target.id === "composer" ? "0" : "-1";
        if (target.getAttribute("tabindex") !== index)
          target.setAttribute("tabindex", index);
      } else {
        for (const node of change.addedNodes)
          if (node instanceof window.Element) update(node);
      }
    }
  });
  update(root);
  observer.observe(root, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["tabindex"],
  });
  return () => observer.disconnect();
}
