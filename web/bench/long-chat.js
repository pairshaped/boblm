async function profileLongChat(page) {
  const baseUrl = new URL(page.url()).origin;
  const results = [];
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  await page.addInitScript(() => {
    window.profileLongTasks = [];
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries())
        window.profileLongTasks.push(entry.duration);
    }).observe({ type: "longtask", buffered: true });
  });
  const metrics = async () =>
    Object.fromEntries(
      (await cdp.send("Performance.getMetrics")).metrics.map((m) => [
        m.name,
        m.value,
      ]),
    );
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    // A fixed slowdown makes regressions easier to see. It does not emulate an iPhone.
    await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
    for (const id of [
      "profile-1000",
      "profile-10000",
      "profile-50000",
      "profile-dense",
    ]) {
      for (let run = 0; run < 3; run++) {
        await page.goto(`${baseUrl}/?chat=${id}`, {
          waitUntil: "domcontentloaded",
        });
        await page.waitForFunction((id) => {
          const el = document.querySelector("#messages");
          return (
            el?.dataset.sessionId === id &&
            el.dataset.historyLoading !== "true" &&
            el.querySelector("[data-row-key]")
          );
        }, id);
        const open = await page.evaluate(() => {
          const m = document.querySelector("#messages");
          const history = performance
            .getEntriesByType("resource")
            .filter((e) => e.name.endsWith("/commands/chatHistory"));
          return {
            readableMs: performance.now(),
            historyHttpMs: history.at(-1)?.duration,
            historyBytes: history.at(-1)?.encodedBodySize,
            rows: m.querySelectorAll("[data-row-key]").length,
            domNodes: document.querySelectorAll("*").length,
            longTasks: [...window.profileLongTasks],
          };
        });
        // Compare five adjacent pages, then scroll inside the retained window.
        const paging = [];
        for (let p = 0; p < 5; p++) {
          const before = await page.evaluate(
            () =>
              document.querySelector("#messages [data-row-key]").dataset.rowKey,
          );
          const start = Date.now();
          await page.evaluate(() => {
            const m = document.querySelector("#messages");
            m.scrollTop = 0;
            m.dispatchEvent(new WheelEvent("wheel", { deltaY: -100 }));
            m.dispatchEvent(new Event("scroll"));
          });
          await page.waitForFunction((before) => {
            const m = document.querySelector("#messages");
            return (
              m.dataset.historyLoading !== "true" &&
              m.querySelector("[data-row-key]").dataset.rowKey !== before
            );
          }, before);
          paging.push(Date.now() - start);
        }
        const beforeScroll = await metrics();
        const scrolling = await page.evaluate(async () => {
          const m = document.querySelector("#messages");
          const gaps = [];
          let last = performance.now();
          for (let i = 0; i < 120; i++) {
            await new Promise(requestAnimationFrame);
            const now = performance.now();
            gaps.push(now - last);
            last = now;
            // Keep away from paging thresholds so this isolates rendering and scroll.
            m.scrollTop = m.scrollHeight / 2 + Math.sin(i / 8) * 300;
          }
          return {
            gaps,
            rows: m.querySelectorAll("[data-row-key]").length,
            domNodes: document.querySelectorAll("*").length,
          };
        });
        const afterScroll = await metrics();
        results.push({
          viewport: viewport.width,
          id,
          run,
          open,
          pagingMs: paging,
          scrolling,
          scrollTaskMs:
            (afterScroll.TaskDuration - beforeScroll.TaskDuration) * 1000,
          scrollLayoutMs:
            (afterScroll.LayoutDuration - beforeScroll.LayoutDuration) * 1000,
          heapBytes: afterScroll.JSHeapUsedSize,
        });
      }
    }
  }
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
  await cdp.detach();
  return results;
}
