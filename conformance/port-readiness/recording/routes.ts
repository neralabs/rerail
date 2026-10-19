/**
 * How a tool-pack gets recorded, for every message that says it needs a recording. Pull request CI takes only a
 * recording port-readiness-record.yml made and attested (fetch.ts), and only someone who can write here starts that
 * run, so in a pull request a maintainer does; a recording made locally goes to this machine's cache, which is enough
 * for the gates here and never for CI. It imports nothing, so the gates, the fetch and the summary can all name it.
 */

/** The label a maintainer puts on a pull request to have port-readiness-record.yml record it. */
export const RECORD_LABEL = "record-port-readiness";

/** The route to a recording of `pack`/`tool` on this machine, whose cache the gates here read and CI never does. */
const localRoute = (pack: string, tool: string): string => `locally, npm run port-readiness:record -- --pack ${pack} --tool ${tool}`;

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
