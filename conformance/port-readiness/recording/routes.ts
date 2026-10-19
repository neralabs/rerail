/**
 * How a tool-pack gets recorded, for every message that says it needs a recording. Pull request CI takes only a
 * recording port-readiness-record.yml made and attested (fetch.ts), and only someone who can write here starts that
 * run, so in a pull request a maintainer does; a recording made locally goes to this machine's cache, which is enough
 * for the gates here and never for CI. The record run takes the recorder's runtimes and its list of tool-packs from
 * main, and only packs/ and the corpus tasks from a pull request, so a change to the runtimes and a new tool-pack are
 * recorded once they are on main, never through the pull request that makes them. It imports nothing, so the gates,
 * the fetch and the summary can all name it.
 */

/** The label a maintainer puts on a pull request to have port-readiness-record.yml record it. */
export const RECORD_LABEL = "record-port-readiness";

/** The route to a recording of `pack`/`tool` on this machine, whose cache the gates here read and CI never does. */
const localRoute = (pack: string, tool: string): string => `locally, npm run port-readiness:record -- --pack ${pack} --tool ${tool}`;

/** How main is recorded, once what the recording needs is there. */
const RECORD_MAIN = "a maintainer then runs port-readiness-record.yml from main with pr and sha empty";

/** Both routes to a recording of `pack`/`tool`: the record workflow in a pull request, the recorder locally. */
export function recordingRoutes(pack: string, tool: string): string {
  return `in a pull request, a maintainer adds the ${RECORD_LABEL} label (or runs port-readiness-record.yml by hand with the pull request's number and head sha);`
    + ` ${localRoute(pack, tool)}`;
}

/**
 * Both routes to a recording of `pack`/`tool` on main, where there is no pull request to label: the record workflow
 * run from main for main, the recorder locally.
 */
export function mainRecordingRoutes(pack: string, tool: string): string {
  return `on main, a maintainer runs port-readiness-record.yml from main with pr and sha empty; ${localRoute(pack, tool)}`;
}

/**
 * The routes to a recording of `pack`/`tool` after a change to the recorder's runtimes: the record run takes them
 * from main, so recording the pull request that changes them records main's again.
 */
export function runtimesRecordingRoutes(pack: string, tool: string): string {
  return "a change to the recorder's runtimes is recorded once it is on main, since the record run takes the runtimes from main:"
    + ` it lands on main first, and ${RECORD_MAIN}; ${localRoute(pack, tool)}`;
}

/**
 * The routes to a first recording of `pack`/`tool`: the record run's legs are main's TOOL_PACKS, so labelling the
 * pull request that adds a tool-pack records nothing for it.
 */
export function newToolPackRoutes(pack: string, tool: string): string {
  return "a new tool-pack is recorded once it is on main, since the record run records only the tool-packs main's port-readiness-record.yml lists in TOOL_PACKS:"
    + ` it lands on main first, with its entry there, and ${RECORD_MAIN}; ${localRoute(pack, tool)}`;
}
