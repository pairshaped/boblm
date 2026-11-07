async function checkStreamingInput(page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  await cdp.send("Network.enable");
  await cdp.send("Network.setCacheDisabled", { cacheDisabled: true });
  await page.goto(new URL("/?chat=profile-dense", page.url()).href);
  await page.waitForFunction(() =>
    document.querySelector("#messages [data-row-key]"),
  );
  const composer = page.locator("#composer");
  await composer.fill("Begin the fake streaming reply");
  await composer.press("Enter");
  await page.waitForFunction(
    () => document.querySelector("#composer").value === "",
  );
  await page.evaluate(() => {
    window.profileInputLatencies = [];
    document.querySelector("#composer").addEventListener("input", () => {
      const start = performance.now();
      requestAnimationFrame(() =>
        window.profileInputLatencies.push(performance.now() - start),
      );
    });
  });
  const draft = "Typing while a reply streams must preserve every character.";
  await composer.pressSequentially(draft, { delay: 50 });
  await page.waitForFunction(() =>
    document
      .querySelector("#messages")
      .textContent.includes("Streaming fixture line 79"),
  );
  const value = await composer.inputValue();
  if (value !== draft) throw new Error("Streaming changed the composer draft");
  const metrics = await page.evaluate(() => {
    const samples = window.profileInputLatencies.sort((a, b) => a - b);
    const composer = document.querySelector("#composer");
    return {
      samples: samples.length,
      inputToAnimationFrameP95Ms: samples[Math.floor(samples.length * 0.95)],
      maximumMs: samples.at(-1),
      focused: document.activeElement === composer,
      caret: composer.selectionStart,
      rows: document.querySelectorAll("#messages [data-row-key]").length,
    };
  });
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
  await cdp.detach();
  if (!metrics.focused || metrics.caret !== draft.length)
    throw new Error("Streaming moved composer focus or caret");
  return { ...metrics, draftPreserved: true };
}
