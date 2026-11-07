async function subagentsDrawerSmoke(page) {
  const smoke = await page.evaluate(() => globalThis.__boblmSmoke ?? null);
  if (!smoke?.url || !smoke?.token || !smoke?.root || !smoke?.child)
    throw new Error(
      "Set globalThis.__boblmSmoke to {url, token, root, child, title, screenshot} before running the subagent drawer smoke test.",
    );
  const { url, token, root, child } = smoke;
  const title = smoke.title ?? "Worker";
  await page.addInitScript((value) => {
    localStorage.setItem("boblm:token", value);
  }, token);
  await page.goto(`${url}/?chat=${encodeURIComponent(root)}`, {
    waitUntil: "domcontentloaded",
  });
  await page.waitForFunction(
    (id) => {
      const messages = document.querySelector("#messages");
      return (
        messages?.dataset.sessionId === id &&
        messages.dataset.historyLoading !== "true" &&
        !!messages.querySelector("[data-row-key]")
      );
    },
    root,
  );
  await page.locator("button.drawer-toggle").click();
  await page.waitForSelector(".subagents");
  await page.locator("#drawer-tab-agents").click();
  await page.locator(".subagents li button", { hasText: title }).first().click();
  await page.waitForFunction(
    (id) => document.querySelector("#subagent-messages")?.dataset.sessionId === id,
    child,
  );
  await page.waitForFunction(
    () =>
      document.querySelector("#subagent-messages")?.dataset.historyLoading !==
      "true",
  );
  const result = await page.evaluate((id) => {
    const header = document.querySelector(".subagent-titlebar");
    const messages = document.querySelector("#subagent-messages");
    return {
      sessionId: messages?.dataset.sessionId ?? null,
      header: header?.textContent ?? "",
      stop: !!header?.querySelector('button[aria-label="Stop"]'),
      archive: !!header?.querySelector('button[aria-label="Archive"]'),
      composer: !!document.querySelector(".subagent-conversation textarea"),
      rows: messages?.querySelectorAll("[data-row-key]").length ?? 0,
      text: messages?.textContent ?? "",
    };
  }, child);
  if (result.sessionId !== child)
    throw new Error(`the drawer opened ${result.sessionId}, not ${child}`);
  if (!result.stop || !result.archive)
    throw new Error("the boblm subagent header is missing Stop or Archive");
  if (result.composer)
    throw new Error("the read-only subagent view unexpectedly has a composer");
  if (!/implementer|reviewer/.test(result.header))
    throw new Error(`the header does not show the role: ${result.header}`);
  if (!/(working|idle|stopped|archived|queued|held|dispatching|stopping)/.test(result.header))
    throw new Error(`the header does not show a status: ${result.header}`);
  if (result.rows < 1)
    throw new Error("the subagent history has no message rows");
  await page.screenshot({
    path: smoke.screenshot ?? "target/subagents-drawer.png",
    fullPage: true,
  });
  return result;
}
