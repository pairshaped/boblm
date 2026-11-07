import {
  h,
  type VNode,
  type ProgramSubscription,
} from "@pairshaped/hypertea/program";
import { limits } from "./generated/contract.ts";
import type { AllowanceReading, SessionInfo } from "./generated/contract.ts";

type MeterAllowance = AllowanceReading;

function meter(
  label: string,
  percent: number | null,
  detail: string,
  control: VNode,
  remaining = false,
) {
  const fill = percent === null ? 0 : Math.min(100, Math.max(0, percent));
  return h(
    "div",
    { class: "usage-meter" },
    h(
      "div",
      { class: "usage-label" },
      control,
      h("span", {}, percent === null ? "—" : `${Math.round(percent)}%`),
    ),
    h(
      "div",
      {
        class: `usage-track${percent === null ? " unknown" : (remaining ? percent <= 100 - limits.usageHighPercent : percent >= limits.usageHighPercent) ? " high" : ""}`,
        role: "progressbar",
        "aria-label": `${label} ${remaining ? "remaining" : "used"}`,
        "aria-description": detail,
        "aria-valuemin": 0,
        "aria-valuemax": 100,
        ...(percent === null
          ? { "aria-valuetext": "Not reported" }
          : { "aria-valuenow": fill }),
      },
      h("span", { style: { width: `${fill}%` } }),
    ),
  );
}
export function usageMeters(
  session: SessionInfo,
  compactEvents: Record<string, unknown>,
  now = Date.now(),
) {
  const usage = session.status.usage;
  const account = session.status.accountUsage;
  const context = usage?.context;
  const contextPercent =
    context &&
    Number.isSafeInteger(context.usedTokens) &&
    Number.isSafeInteger(context.windowTokens) &&
    context.usedTokens >= 0 &&
    context.windowTokens > 0
      ? (100 * context.usedTokens) / context.windowTokens
      : null;
  const fiveHour = account?.fiveHour;
  const weekly = account?.weekly;
  const creditBalance = account?.credits?.balance;
  const creditsAvailable = account?.credits ? creditBalance != null : false;
  return h(
    "div",
    { class: "usage-meters", "aria-label": "Agent usage" },
    meter(
      "Context",
      contextPercent,
      contextPercent === null
        ? "Context usage has not been reported yet."
        : `${context!.usedTokens.toLocaleString()} of ${context!.windowTokens.toLocaleString()} context tokens used.`,
      h(
        "div",
        {},
        h(
          "button",
          {
            type: "button",
            class: "usage-action",
            tabindex: -1,
            popovertarget: `context-compact-${session.sessionId}`,
          },
          "Context",
        ),
        h(
          "div",
          {
            id: `context-compact-${session.sessionId}`,
            popover: "auto",
            class: "usage-popover",
            role: "dialog",
            "aria-label": "Compact context",
          },
          h("strong", {}, "Compact this chat?"),
          h(
            "div",
            { class: "usage-popover-actions" },
            h(
              "button",
              {
                type: "button",
                tabindex: -1,
                popovertarget: `context-compact-${session.sessionId}`,
                popovertargetaction: "hide",
              },
              "Cancel",
            ),
            h(
              "button",
              {
                ...compactEvents,
                type: "button",
                tabindex: -1,
                popovertarget: `context-compact-${session.sessionId}`,
                popovertargetaction: "hide",
              },
              "Compact",
            ),
          ),
        ),
      ),
    ),
    fiveHour
      ? allowanceMeter(
          "Five-hour",
          "five-hour",
          fiveHour,
          session.sessionId,
          now,
        )
      : null,
    session.agent === "pi"
      ? null
      : allowanceMeter("Weekly", "weekly", weekly, session.sessionId, now),
    creditsAvailable &&
      creditBalance != null &&
      Number.isFinite(creditBalance) &&
      creditBalance > 0
      ? h(
          "div",
          { class: "usage-credits", "aria-label": "Credits remaining" },
          h("span", {}, "Credits"),
          h("span", {}, Math.floor(creditBalance).toLocaleString()),
        )
      : null,
  );
}
function allowanceMeter(
  label: string,
  key: string,
  usage: MeterAllowance | null | undefined,
  sessionId: string,
  now: number,
) {
  const remaining =
    usage &&
    Number.isFinite(usage.usedPercent) &&
    usage.usedPercent >= 0 &&
    (usage.resetsAt == null || usage.resetsAt * 1000 > now)
      ? Math.max(0, 100 - usage.usedPercent)
      : null;
  const reset = usage?.resetsAt;
  const resetText =
    reset != null && Number.isFinite(reset) && reset * 1000 > now
      ? `Resets ${new Date(reset * 1000).toLocaleString(undefined, { ...(key === "weekly" ? { dateStyle: "full" as const } : {}), timeStyle: "short" })}.`
      : "The next reset time has not been reported.";
  return meter(
    label,
    remaining,
    remaining == null
      ? `${label} allowance has not been reported for the current window.`
      : `${remaining.toLocaleString(undefined, { maximumFractionDigits: 2 })}% remaining. ${resetText}`,
    h(
      "div",
      {},
      h(
        "button",
        {
          type: "button",
          class: "usage-action",
          tabindex: -1,
          popovertarget: `${key}-reset-${sessionId}`,
        },
        label,
      ),
      h(
        "div",
        {
          id: `${key}-reset-${sessionId}`,
          popover: "auto",
          class: "usage-popover",
        },
        resetText,
      ),
    ),
    true,
  );
}
/** Expire an old allowance observation without inventing a fresh 0% value. */
export function usageResetSubscription<Msg extends Readonly<{ type: string }>>(
  session: SessionInfo,
  expired: Msg,
  now = Date.now(),
): ProgramSubscription<Msg> | null {
  const account = session.status.accountUsage;
  const resets = [
    account?.fiveHour?.resetsAt,
    session.agent === "pi" ? null : account?.weekly?.resetsAt,
  ].filter(
    (reset): reset is number =>
      reset != null && Number.isFinite(reset) && reset * 1000 > now,
  );
  if (!resets.length) return null;
  const reset = Math.min(...resets);
  return {
    key: `usage-reset:${session.sessionId}:${reset}`,
    subscribe: (dispatch) => {
      const timer = setTimeout(
        () => dispatch(expired),
        Math.min(2_147_483_647, reset * 1000 - now),
      );
      return () => clearTimeout(timer);
    },
  };
}
