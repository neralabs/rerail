/**
 * The page's routes, as pure functions of the URL (the shell applies them; `test/ui/ui-routes.test.ts` covers them).
 *
 * The query names the repository the page shows (`?repository=<key>&checkout=<path>`) and the selected session
 * (`sessionId=<id>`, with the `host` it ran in); every route keeps that scope. The hash names the view:
 * `#overview` (the default), `#pack/<packId>`, `#created` (rules created by you), `#packs` (the catalog),
 * `#packs?review=1` (the review before starting), `#create`, `#history` (`?rule=<id>` narrows it to one rule,
 * `?outcome=steered|logged` sets its chip), `#sessions` (`?filter=violations|in-progress` sets its chip), `#settings`,
 * `#approvals` (the rules waiting for a person's approval), and `#rule/<id>`: a rule's sheet over the current view
 * (the coding agent's steer links here). Links from steers, the CLI and the status line written before v8 keep
 * working: `#rules` is Overview, `#activity` History, `#rule-packs` the catalog and `#rule-packs?review=1` the review.
 */

/**
 * @typedef {"overview" | "pack" | "created" | "packs" | "review" | "create" | "history" | "sessions" | "settings" | "approvals"} ViewName
 * @typedef {"all" | "steered" | "logged"} Outcome   History by rule's chips.
 * @typedef {"all" | "corr" | "open"} SessionFilter History by session's chips: All, With violations, In progress.
 * @typedef {object} Route
 * @property {ViewName | null} view  `null` for a rule link: the sheet opens over whatever view shows.
 * @property {string} [pack]         The pack a `pack` route shows.
 * @property {string} [filterRule]   The rule History is narrowed to.
 * @property {Outcome} [outcome]     History by rule's chip (`history` routes only).
 * @property {SessionFilter} [sessionFilter] History by session's chip (`sessions` routes only).
 * @property {string} [rule]         The rule whose sheet is open.
 * @property {string} [session]      The session Sessions selects.
 * @property {boolean} [jump]        Sessions scrolls to the selected session's first violation.
 */

/** Hashes that name a view by themselves, the legacy ones included. */
const PLAIN = {
  overview: "overview", rules: "overview",
  created: "created", create: "create", sessions: "sessions", settings: "settings", approvals: "approvals",
  packs: "packs", "rule-packs": "packs",
  history: "history", activity: "history",
};

const decode = text => {
  try { return decodeURIComponent(text); } catch { return ""; }
};
/** Ids keep `:` readable in the address bar (`typescript:no-any`), as steer links write them. */
const encode = id => encodeURIComponent(id).replace(/%3A/gi, ":");

/** History by rule's chips as the hash spells them; All has no parameter. */
const OUTCOMES = { steered: true, logged: true };
/** History by session's chips: the hash's word for each, and back. */
const SESSION_FILTER_PARAM = { corr: "violations", open: "in-progress" };
const SESSION_FILTER = { violations: "corr", "in-progress": "open" };
/** `table[key]` for a key the table itself holds; a URL's text never reaches inherited names like "constructor". */
const own = (table, key) => (typeof key === "string" && Object.hasOwn(table, key) ? table[key] : undefined);

/** The route a hash (with or without its `#`) names; anything unknown is Overview. */
export function parseRoute(hash) {
  const raw = hash.startsWith("#") ? hash.slice(1) : hash;
  // The rest of a rule or pack link is the id, `?` included.
  if (raw.startsWith("rule/")) {
    const rule = decode(raw.slice(5));
    return rule === "" ? { view: null } : { view: null, rule };
  }
  if (raw.startsWith("pack/")) {
    const pack = decode(raw.slice(5));
    return pack === "" ? { view: "overview" } : { view: "pack", pack };
  }
  const at = raw.indexOf("?");
  const path = at < 0 ? raw : raw.slice(0, at);
  const params = new URLSearchParams(at < 0 ? "" : raw.slice(at + 1));
  const view = PLAIN[path] ?? "overview";
  if (view === "packs" && params.get("review") === "1") return { view: "review" };
  if (view === "history") {
    const filterRule = params.get("rule"), outcome = params.get("outcome");
    return { view, ...(filterRule ? { filterRule } : {}), ...(own(OUTCOMES, outcome) ? { outcome } : {}) };
  }
  const sessionFilter = own(SESSION_FILTER, params.get("filter"));
  if (view === "sessions" && sessionFilter) return { view, sessionFilter };
  return { view };
}

/** `key=value` pairs (values encoded as ids are), joined into a hash's query; "" without any. */
function query(pairs) {
  const text = pairs.filter(([, v]) => v !== undefined).map(([k, v]) => `${k}=${encode(v)}`).join("&");
  return text === "" ? "" : `?${text}`;
}

/** The hash (without `#`) the page writes for `route`: the open rule's link while its sheet shows, else the view's. */
export function hashOf(route) {
  if (route.rule) return `rule/${encode(route.rule)}`;
  switch (route.view) {
    case "pack": return `pack/${encode(route.pack ?? "")}`;
    case "review": return "packs?review=1";
    case "history": return `history${query([["outcome", own(OUTCOMES, route.outcome) ? route.outcome : undefined], ["rule", route.filterRule || undefined]])}`;
    case "sessions": return `sessions${query([["filter", own(SESSION_FILTER_PARAM, route.sessionFilter)]])}`;
    default: return route.view ?? "overview";
  }
}

/**
 * The route a fresh page opens on. A session link (`?sessionId=<id>`) opens Sessions on that session whatever the
 * hash says, a rule link's sheet over it included; a rule link alone opens over Overview.
 */
export function bootRoute(search, hash) {
  const route = parseRoute(hash);
  const session = new URLSearchParams(search).get("sessionId");
  if (session) return route.rule === undefined ? { view: "sessions", session } : { view: "sessions", session, rule: route.rule };
  return route.view === null ? { ...route, view: "overview" } : route;
}

/**
 * The page's URL showing `route`, from the current `href`: the repository and checkout stay as they are; `sessionId`
 * is set to `session`, or dropped with `null`, and the `host` that came with a session link goes once another
 * session (or none) is shown.
 */
export function urlFor(href, route, session = null) {
  const url = new URL(href);
  if (url.searchParams.get("sessionId") !== session) url.searchParams.delete("host");
  if (session) url.searchParams.set("sessionId", session); else url.searchParams.delete("sessionId");
  url.hash = hashOf(route);
  return url.href;
}

/** The URL showing repository `key`'s Overview, or with `null` the page that asks for a repository. */
export function repositoryUrl(href, key) {
  const url = new URL(href);
  for (const name of ["repository", "checkout", "sessionId", "host"]) url.searchParams.delete(name);
  if (key !== null) url.searchParams.set("repository", key);
  url.hash = hashOf({ view: "overview" });
  return url.href;
}
