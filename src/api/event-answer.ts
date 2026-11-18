/**
 * What the daemon's answer to a hook's event says beside its deliveries (`EventAnswerFacts`): the facts the hook would
 * otherwise read back over HTTP right after the answer, made from what the daemon holds as it answers: its dashboard's
 * origin, and what the runtime that took the event holds of its session (`HeldSession`). Nothing here reads, so the
 * answer waits for none of it.
 */
import type { AdapterDelivery, AdapterEvent, EventAnswerFacts, HeldSession } from "../engine/adapter-contract.js";
import { LOOPBACK_HOSTNAMES } from "./http.js";
import { rulesDashboardUrl } from "./presentation-contracts.js";

/**
 * The origin of the daemon's dashboard that a hook may link to, or null where it listens on a host a browser should not
 * be sent to (`0.0.0.0`), as the hook rules for the daemon.json URL it reads (`verifyDaemon`).
 */
function linkableOrigin(origin: string): URL | null {
  const url = new URL(origin);
  return LOOPBACK_HOSTNAMES.has(url.hostname) ? url : null;
}

/** The dashboard a hook links `checkout`'s note and steers into, as it builds it from `/api/repository` (`steerDashboard`). */
function checkoutDashboard(origin: URL, checkout: HeldSession["checkout"]): string {
  const url = new URL(rulesDashboardUrl(origin, checkout));
  url.hash = "";
  return url.href;
}

/** Whether the hook links something of the answer to `event`: the session-start note, a rule or a conflict. */
function linksSomething(event: AdapterEvent, deliveries: readonly AdapterDelivery[]): boolean {
  return event.kind === "session-start" || deliveries.some(delivery => delivery.rules.length > 0 || delivery.conflict !== undefined);
}

/**
 * The facts of the answer to `event` from a daemon whose dashboard is at `origin`: the dashboard where the hook links
 * something (null where the daemon has none a browser may open), and at a session's start what its note says of its
 * rules. Nothing where no runtime holds the event's session (`session` undefined); the hook then reads both after the
 * answer, as from a daemon older than these facts.
 */
export function eventAnswerFacts(
  origin: string, event: AdapterEvent, deliveries: readonly AdapterDelivery[], session: HeldSession | undefined,
): EventAnswerFacts {
  if (session === undefined) return {};
  const linkable = linkableOrigin(origin);
  return {
    ...(linksSomething(event, deliveries) ? { dashboard: linkable === null ? null : checkoutDashboard(linkable, session.checkout) } : {}),
    ...(session.sessionNote === undefined ? {} : { sessionNote: session.sessionNote }),
  };
}
