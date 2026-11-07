async function checkChatResume(page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Network.enable");
  await cdp.send("Network.setCacheDisabled", { cacheDisabled: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(new URL("/?chat=profile-1000", page.url()).href);
  await page.waitForFunction(() =>
    document.querySelector("#messages [data-row-key]"),
  );
  await page.locator("#composer").fill("Keep this unfinished reply");
  const calls = [];
  const track = (request) => {
    if (
      request.url().endsWith("/commands/chatHistory") ||
      request.url().includes("/profile-1000/events")
    )
      calls.push({
        url: new URL(request.url()).pathname,
        body: request.postData(),
        cursor: request.headers()["last-event-id"],
      });
  };
  page.on("request", track);
  await page.evaluate(() => {
    window.profileHidden = true;
    window.profileClock = 0;
    const now = Date.now.bind(Date);
    Date.now = () => now() + window.profileClock;
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => (window.profileHidden ? "hidden" : "visible"),
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  // Represents a second device continuing this chat while the phone is away.
  await page.evaluate(async () => {
    const response = await fetch("/commands/sendMessage", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: "Bearer " + localStorage.getItem("boblm:token"),
      },
      body: JSON.stringify({
        sessionId: "profile-1000",
        messageId: crypto.randomUUID(),
        content: "Browser resume fixture",
      }),
    });
    if (!response.ok)
      throw new Error("Fixture send failed: " + response.status);
  });
  await page.waitForTimeout(8500);
  const start = Date.now();
  await page.evaluate(() => {
    window.profileClock = 16 * 60 * 1000;
    window.profileHidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(750);
  const result = {
    calls,
    resumeObservedMs: Date.now() - start,
    draft: await page.locator("#composer").inputValue(),
    latestText: await page.locator("#messages").innerText(),
  };
  page.off("request", track);
  if (result.draft !== "Keep this unfinished reply")
    throw new Error("Composer draft was lost");
  if (
    !calls.some(
      (c) => c.body && JSON.parse(c.body).sessionId === "profile-1000",
    )
  )
    throw new Error(
      "Returning after sixteen minutes replayed the old stream instead of requesting a recent snapshot",
    );
  if (!result.latestText.includes("Streaming fixture line 79"))
    throw new Error("Latest reply is missing");
  await cdp.detach();
  return {
    calls,
    resumeObservedMs: result.resumeObservedMs,
    draftPreserved: true,
    latestReplyVisible: true,
  };
}
