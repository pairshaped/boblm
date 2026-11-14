import { h, bindEvents } from "@pairshaped/hypertea/program";
import type { Model, Msg } from "./app.ts";
import type {
  AccountUsageInfo,
  AllowanceReading,
} from "./generated/contract.ts";
import { meter } from "./usage.ts";
const on = bindEvents<Msg>();

/** "2d 4h", "3h 5m" or "12m"; never finer than a minute. */
function span(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60_000));
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  const rest = minutes % 60;
  if (days > 0) return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  if (hours > 0) return rest > 0 ? `${hours}h ${rest}m` : `${hours}h`;
  return `${rest}m`;
}

function age(observedAt: number, now: number): string {
  const ms = now - observedAt;
  return ms < 60_000 ? "as of just now" : `as of ${span(ms)} ago`;
}

function allowance(
  label: string,
  _weekly: boolean,
  reading: AllowanceReading | null | undefined,
  now: number,
) {
  if (!reading || !Number.isFinite(reading.usedPercent))
    return h(
      "div",
      { class: "account-window" },
      meter(
        label,
        null,
        `${label} allowance has not been reported.`,
        h("span", {}, label),
        true,
      ),
      h("small", {}, "Not reported yet."),
    );
  const reset = reading.resetsAt;
  // An expired window is unavailable until the provider reports the next one (ADR 0037).
  const expired = reset != null && reset * 1000 <= now;
  const remaining = expired ? null : Math.max(0, 100 - reading.usedPercent);
  const when =
    reset == null
      ? "The next reset time has not been reported"
      : expired
        ? "This window has reset, waiting for a new reading"
        : `Resets in ${span(reset * 1000 - now)}`;
  const local =
    reset != null && !expired
      ? new Date(reset * 1000).toLocaleString(undefined, {
          weekday: "short",
          month: "short",
          day: "numeric",
          hour: "numeric",
          minute: "2-digit",
        })
      : "";
  return h(
    "div",
    { class: "account-window" },
    meter(
      label,
      remaining,
      remaining === null
        ? `${when}.`
        : `${remaining.toLocaleString(undefined, { maximumFractionDigits: 2 })}% remaining. ${when}.`,
      h("span", {}, `${label} remaining`),
      true,
    ),
    h("small", {}, local ? `${when}, ${local}` : when),
    h("small", { class: "account-window-age" }, age(reading.observedAt, now)),
  );
}

function account(info: AccountUsageInfo, now: number) {
  const credits = info.credits?.balance;
  const body =
    info.kind === "api"
      ? h("p", {}, "Allowance is not reported for API accounts.")
      : !info.weekly && !info.credits
        ? h("p", {}, "No readings yet.")
        : [
            // The five-hour window is not shown: these are top plans, so only the weekly one matters.
            allowance("Weekly", true, info.weekly, now),
            credits != null && Number.isFinite(credits) && credits > 0
              ? h(
                  "div",
                  { class: "usage-credits", "aria-label": "Credits remaining" },
                  h("span", {}, "Credits"),
                  h(
                    "span",
                    {},
                    credits.toLocaleString(undefined, {
                      maximumFractionDigits: 2,
                    }),
                  ),
                )
              : null,
          ];
  return h(
    "section",
    { key: info.label, class: "account-usage-account" },
    h("h3", {}, info.label),
    body,
  );
}

export function accountUsageContent(model: Model, now = Date.now()) {
  return h(
    "div",
    { class: "subagents-body account-usage" },
    model.accountUsageError
      ? h(
          "div",
          {},
          h("p", { role: "alert", class: "error" }, model.accountUsageError),
          h(
            "button",
            {
              type: "button",
              onClick: on.clicked({ type: "reloadAccountUsage" }),
            },
            "Try again",
          ),
        )
      : null,
    model.accountUsageLoading
      ? h("p", { role: "status" }, "Loading usage…")
      : null,
    model.accountUsage.map((info) => account(info, now)),
  );
}
