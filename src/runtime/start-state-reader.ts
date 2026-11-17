/**
 * The checkout's start state read from the database, for the Claude Code status line when no daemon answers. The build
 * bundles this module over its compiled output (`build:ts`), Drizzle included: loaded unbundled, the read resolved
 * drizzle-orm at run time, whose 179 KB `exports` map Node 24 parses again for every module it loads from the package,
 * and a status-line run peaked at 285 MB.
 */
export { readOnlyStartState } from "./repository-start.js";
