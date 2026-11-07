// Push only. App assets keep their normal HTTP cache and update behavior.
self.addEventListener("push", (event) => {
  let payload;
  try {
    payload = event.data?.json();
  } catch {
    payload = null;
  }
  const url = new URL("/", self.location.origin);
  if (typeof payload?.sessionId === "string")
    url.searchParams.set("chat", payload.sessionId);
  if (typeof payload?.questionId === "string")
    url.searchParams.set("question", payload.questionId);
  event.waitUntil(
    self.registration.showNotification(
      typeof payload?.title === "string" ? payload.title : "BobLM",
      {
        body:
          typeof payload?.body === "string"
            ? payload.body
            : "A chat needs your attention.",
        tag: typeof payload?.id === "string" ? payload.id : "boblm",
        icon: "/icon-192.png",
        data: { url: url.href },
      },
    ),
  );
});
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = new URL(
    event.notification.data?.url || "/",
    self.location.origin,
  );
  if (url.origin !== self.location.origin) return;
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({
        type: "window",
        includeUncontrolled: true,
      });
      const existing = windows.find(
        (client) => new URL(client.url).origin === self.location.origin,
      );
      if (existing) {
        await existing.navigate(url.href);
        await existing.focus();
      } else {
        await self.clients.openWindow(url.href);
      }
    })(),
  );
});
