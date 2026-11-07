import {
  emptySubscription,
  type ProgramSubscription,
} from "@pairshaped/hypertea/program";
import type { Model, Msg } from "./app.ts";
import {
  limits,
  parseEmpty,
  parsePushConfigReply,
  parseChatPresenceReply,
} from "./generated/contract.ts";
import { postCommand } from "./http.ts";
let registration: ServiceWorkerRegistration | null = null;
let publicKey: string | null = null;

function supported(): boolean {
  return (
    window.isSecureContext &&
    "serviceWorker" in navigator &&
    "PushManager" in window &&
    "Notification" in window
  );
}
async function saveSubscription(subscription: PushSubscription, token: string) {
  const json = subscription.toJSON();
  const outcome = await postCommand(
    "/commands/subscribePush",
    {
      endpoint: subscription.endpoint,
      p256dh: json.keys?.p256dh,
      auth: json.keys?.auth,
    },
    token,
    parseEmpty,
  );
  if (outcome.kind !== "replied")
    throw new Error(
      outcome.kind === "rejected"
        ? outcome.error.message
        : "The push subscription reply was lost. Try enabling notifications again to register the same subscription.",
    );
}
/** Permission is requested only from the owner's Enable notifications action. */
export async function enablePush(token: string): Promise<void> {
  if (!supported() || !registration || !publicKey)
    throw new Error(
      "Push is unavailable here. On iPhone, add this HTTPS app to your Home Screen and open it there first.",
    );
  const permission = await Notification.requestPermission();
  if (permission !== "granted")
    throw new Error(
      "Notification permission was not granted. Enable it in this browser's or device's notification settings, then try again.",
    );
  const bytes = Uint8Array.from(
    atob(publicKey.replace(/-/g, "+").replace(/_/g, "/")),
    (c) => c.charCodeAt(0),
  );
  const subscription =
    (await registration.pushManager.getSubscription()) ??
    (await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: bytes,
    }));
  await saveSubscription(subscription, token);
}
export async function disablePush(token: string): Promise<void> {
  if (!supported()) return;
  const current =
    registration ?? (await navigator.serviceWorker.getRegistration("/"));
  const subscription = await current?.pushManager.getSubscription();
  if (!subscription) return;
  // Browser unsubscription still prevents delivery if the server reply was lost.
  if (!(await subscription.unsubscribe()))
    throw new Error(
      "This browser could not unsubscribe from push. Check notification permissions in the device settings.",
    );
  const outcome = await postCommand(
    "/commands/unsubscribePush",
    { endpoint: subscription.endpoint },
    token,
    parseEmpty,
  );
  if (outcome.kind !== "replied")
    throw new Error(
      "Push was disabled in this browser, but the server reply was lost. Its old subscription will be removed when the push service reports it expired.",
    );
}
export function pushSubscription(model: Model): ProgramSubscription<Msg> {
  if (!model.token || model.connectionPending || !supported())
    return emptySubscription<Msg>("slot:push");
  return {
    key: "push",
    subscribe: (dispatch) => {
      let alive = true;
      void (async () => {
        const result = await postCommand(
          "/commands/pushConfig",
          {},
          model.token,
          parsePushConfigReply,
        );
        if (!alive || result.kind !== "replied" || !result.reply.publicKey)
          return;
        publicKey = result.reply.publicKey;
        registration = await navigator.serviceWorker.register("/sw.js");
        await navigator.serviceWorker.ready;
        const subscription = await registration.pushManager.getSubscription();
        if (subscription) await saveSubscription(subscription, model.token);
        if (alive)
          dispatch({
            type: "pushState",
            state: subscription ? "enabled" : "available",
          });
      })().catch((error) => {
        if (alive)
          dispatch({
            type: "pushState",
            state: "available",
            error:
              error instanceof Error
                ? error.message
                : "Push setup failed. Check the connection and try again.",
          });
      });
      return () => {
        alive = false;
      };
    },
  };
}
export function presenceSubscription(model: Model): ProgramSubscription<Msg> {
  if (!model.token || model.connectionPending || !model.active)
    return emptySubscription<Msg>("slot:presence");
  return {
    key: `presence:${model.active}`,
    subscribe: (dispatch) => {
      let clientId = crypto.randomUUID();
      let alive = true;
      let generation = 0;
      const seen = new Set<string>();
      const send = async (sessionId: string | null) => {
        const current = ++generation;
        const result = await postCommand(
          "/commands/chatPresence",
          { clientId, sessionId, reloadCapable: true },
          model.token,
          parseChatPresenceReply,
        );
        if (!alive || current !== generation) return;
        if (result.kind !== "replied") {
          // A new identity cannot receive an old reload after a failed connection.
          clientId = crypto.randomUUID();
          return;
        }
        const id = result.reply.reloadId;
        if (
          id &&
          sessionId &&
          !seen.has(id) &&
          document.visibilityState === "visible" &&
          document.hasFocus()
        ) {
          seen.add(id);
          dispatch({ type: "reloadPage", sessionId });
        }
      };
      const renew = () =>
        send(
          document.visibilityState === "visible" && document.hasFocus()
            ? model.active
            : null,
        );
      renew();
      const timer = setInterval(renew, limits.notificationHeartbeatMs);
      window.addEventListener("focus", renew);
      window.addEventListener("blur", renew);
      document.addEventListener("visibilitychange", renew);
      return () => {
        alive = false;
        clearInterval(timer);
        window.removeEventListener("focus", renew);
        window.removeEventListener("blur", renew);
        document.removeEventListener("visibilitychange", renew);
        send(null);
      };
    },
  };
}
