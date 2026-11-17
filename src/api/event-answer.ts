/**
 * What the daemon's answer to a hook's event says beside its deliveries (`EventAnswerFacts`): the facts the hook would
 * otherwise read back over HTTP right after the answer, read here from the same operations those reads call.
 */
import type { AdapterDelivery, AdapterEvent, AdapterSession, EventAnswerFacts, SessionNoteRules } from "../engine/adapter-contract.js";
import { checkoutRealpath } from "../engine/catalog/checkout-control.js";
import type { ApiCaller } from "../engine/catalog/repository-model.js";
import { invokeOperation, type RerailApiStores } from "./operation.js";
import { operations } from "./operations.js";
import { presentationReadSchema, rulesDashboardUrl, type PresentedSession, type RulesDashboardScope } from "./presentation-contracts.js";
import { resolveTarget } from "./repository-operations.js";

/** The hook's reads come through `/api`, whose callers are people. */
const PERSON: ApiCaller = { kind: "person" };

/** The hosts a link may send a browser to, as the hook rules for the daemon.json URL it reads (`verifyDaemon`). */
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "[::1]", "localhost"]);

/**
 * The origin of the dashboard at the daemon's own `url` (`http(s)://<host>:<port>/`) that a hook may link to, or null
 * where it listens on a host a browser should not be sent to (`0.0.0.0`).
 */
export function linkableDashboardOrigin(url: string): URL | null {
  const origin = new URL(url);
  return LOOPBACK_HOSTS.has(origin.hostname) ? origin : null;
}

/** The repository and checkout `/api/repository?checkout=` names for `cwd`; null where Rerail does not know it. */
function dashboardScope(stores: RerailApiStores, cwd: string): RulesDashboardScope | null {
  try {
    const target = resolveTarget(stores, { checkout: cwd }, PERSON, false);
    return { repositoryKey: target.repository.repositoryKey, checkoutPath: target.checkout?.checkoutPath ?? null };
  } catch {
    return null;
  }
}

/** The dashboard a hook links `cwd`'s note and steers into, as it builds it from `/api/repository` (`steerDashboard`). */
function checkoutDashboard(stores: RerailApiStores, origin: URL, cwd: string): string {
  const url = new URL(rulesDashboardUrl(origin, dashboardScope(stores, cwd)));
  url.hash = "";
  return url.href;
}

const presentationGet = operations.find(operation => operation.id === "presentation_get")!;

/**
 * What the session-start note says about `session`'s rules, from the presentation the hook reads on demand
 * (`presentedSessionNoteRules`): not set up, the count on the session's footer, or no count where the repository is
 * stopped or the session is not recorded as this one.
 */
async function sessionNoteRules(stores: RerailApiStores, { host, sessionId, cwd }: AdapterSession): Promise<SessionNoteRules> {
  // The hook asks about the folder it resolved, and the presentation finds the checkout from there.
  const input = { repository: checkoutRealpath(cwd), host, sessionId, onDemand: true };
  const read = presentationReadSchema.parse(await invokeOperation(presentationGet, stores, input, PERSON));
  if (read.repositoryState === "not-set-up") return { setUp: false };
  return { setUp: true, ruleCount: read.repositoryState === "stopped" ? null : sessionRuleCount(read.context?.session, { host, sessionId }) };
}

/** The rules on for the session the presentation answered with, once it is this one; null otherwise. */
function sessionRuleCount(answered: PresentedSession | null | undefined, { host, sessionId }: Pick<AdapterSession, "host" | "sessionId">): number | null {
  if (answered === null || answered === undefined || answered.session.sessionId !== sessionId || answered.session.host !== host) return null;
  return answered.hud.activeRuleCount;
}

/**
 * The dashboard of the answer to `event` where the hook links something of it (the session-start note, a rule or a
 * conflict): null where the daemon has none to open (`origin`). Nothing for an answer with nothing to link.
 */
function answerDashboard(
  stores: RerailApiStores, origin: URL | null, event: AdapterEvent & { session: AdapterSession }, deliveries: readonly AdapterDelivery[],
): EventAnswerFacts {
  const links = event.kind === "session-start" || deliveries.some(delivery => delivery.rules.length > 0 || delivery.conflict !== undefined);
  if (!links) return {};
  return { dashboard: origin === null ? null : checkoutDashboard(stores, origin, event.session.cwd) };
}

/**
 * The facts of the answer to `event`: the dashboard where the hook links something (`origin` null where the daemon
 * has none to open), and at a session's start what its note says of its rules. Empty for an event that names no
 * session, and undefined where a read fails: the hook then reads as it did before the answer carried them.
 */
export async function eventAnswerFacts(
  stores: RerailApiStores, origin: URL | null, event: AdapterEvent, deliveries: readonly AdapterDelivery[],
): Promise<EventAnswerFacts | undefined> {
  const session = event.session;
  if (session === undefined) return {};
  try {
    return {
      ...answerDashboard(stores, origin, { ...event, session }, deliveries),
      ...(event.kind === "session-start" ? { sessionNote: await sessionNoteRules(stores, session) } : {}),
    };
  } catch {
    return undefined;
  }
}
