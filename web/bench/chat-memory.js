async function measureChatMemory(page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Network.enable");
  await cdp.send("Network.setCacheDisabled", { cacheDisabled: true });
  await cdp.send("Performance.enable");
  await page.goto(new URL("/?chat=profile-dense", page.url()).href);
  await page.waitForFunction(() =>
    document.querySelector("#messages [data-row-key]"),
  );
  for (let p = 0; p < 5; p++) {
    const first = await page
      .locator("#messages [data-row-key]")
      .first()
      .getAttribute("data-row-key");
    await page.evaluate(() => {
      const m = document.querySelector("#messages");
      m.scrollTop = 0;
      m.dispatchEvent(new WheelEvent("wheel", { deltaY: -100 }));
      m.dispatchEvent(new Event("scroll"));
    });
    await page.waitForFunction((first) => {
      const m = document.querySelector("#messages");
      return (
        m.dataset.historyLoading !== "true" &&
        m.querySelector("[data-row-key]").dataset.rowKey !== first
      );
    }, first);
  }
  const measure = async () => {
    await cdp.send("HeapProfiler.collectGarbage");
    const metrics = Object.fromEntries(
      (await cdp.send("Performance.getMetrics")).metrics.map((m) => [
        m.name,
        m.value,
      ]),
    );
    return {
      heapBytes: metrics.JSHeapUsedSize,
      ...(await cdp.send("Memory.getDOMCounters")),
      ...(await page.evaluate(() => ({
        rows: document.querySelectorAll("#messages [data-row-key]").length,
        liveDomNodes: document.querySelectorAll("*").length,
      }))),
    };
  };
  const before = await measure();
  let release;
  let requested = false;
  await page.route("**/commands/chatHistory", async (route) => {
    requested = true;
    await new Promise((resolve) => (release = resolve));
    await route.continue();
  });
  await page.evaluate(() => {
    const now = Date.now.bind(Date);
    Date.now = () => now() + 16 * 60 * 1000;
    window.dispatchEvent(new Event("focus"));
  });
  await page.waitForFunction(
    () =>
      document.querySelector("#messages").dataset.historyLoading === "true" &&
      !document.querySelector("#messages [data-row-key]"),
  );
  const cleared = await measure();
  release();
  await page.waitForFunction(
    () =>
      document.querySelector("#messages").dataset.historyLoading !== "true" &&
      document.querySelector("#messages [data-row-key]"),
  );
  const repopulated = await measure();
  await page.unroute("**/commands/chatHistory");
  await cdp.detach();
  return { before, cleared, repopulated, snapshotRequested: requested };
}
