/**
 * The views, by the name `ui.view` holds (`routes.js`), and the pages shown in their place when they cannot show.
 *
 * Every module here exports:
 * - `render(root)`: draw into `#content` (`root`). The shell empties `root` whenever the page changes, so a view
 *   may keep its own built nodes between renders and re-attach them;
 * - `needs`: the names of what a poll loads for it (`NEEDS` in `poll.js`), beyond the sidebar's and an open sheet's;
 * and may export:
 * - `actions`: `data-act` name → `(id, element)` for clicks inside `#content`, looked up before `commonActions`;
 * - `enter(route)`: run when the view is entered (and again for a route carrying a History rule or a session);
 * - `afterPoll()`: run after each poll while the view shows;
 * - `onInput(element)`, `onChange(element)`: input and change events inside `#content`; `onResize()`.
 */
import { data, ui } from "../state.js";
import { SHEET_NEEDS } from "../sheet.js";
import * as overview from "./overview.js";
import * as pack from "./pack.js";
import * as packs from "./packs.js";
import * as create from "./create.js";
import * as history from "./history.js";
import * as sessions from "./sessions.js";
import * as settings from "./settings.js";
import * as noRepo from "./no-repo.js";
import * as missing from "./missing.js";
import * as notSetUp from "./not-set-up.js";

export const VIEWS = { overview, pack, created: pack, packs, review: packs, create, history, sessions, settings };

const PAGES = { "no-repo": noRepo, missing, "not-set-up": notSetUp };

/** Views a repository that is not set up shows its Not set up page in place of: they list rules, and it has none. */
const NEED_SET_UP = new Set(["overview", "pack", "created"]);

/**
 * What `#content` shows: the view, or the page in its place. Settings are the machine's and always show; every other
 * view needs a repository the daemon knows.
 */
export function pageKey() {
  if (ui.view === "settings") return "settings";
  if (ui.scope === null) return "no-repo";
  if (data.missing) return "missing";
  if (NEED_SET_UP.has(ui.view) && data.repository !== null && !data.repository.setUp) return "not-set-up";
  return ui.view;
}

/** The module that draws page `key`. */
export const pageModule = (key = pageKey()) => PAGES[key] ?? VIEWS[key];

/** The sidebar lists the repository's packs with how many of their rules are on, on every view. */
const SIDEBAR_NEEDS = ["rules", "packs"];

/** What a poll loads for the page now (`setNeeds` in `poll.js`): the sidebar's, the view's and an open sheet's. */
export function currentNeeds() {
  if (ui.scope === null) return [];
  return [...SIDEBAR_NEEDS, ...VIEWS[ui.view].needs, ...(ui.panel === null ? [] : SHEET_NEEDS)];
}
