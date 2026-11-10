import { ADAPTER_CAPABILITIES, steerBlock, type RerailHost, type SteerRule } from "../../src/engine/index.js";

/** An edit's steer for one finding of `ruleId`, as the code-rule enforcer renders it. */
export function editSteer(ruleId: string, message: string, location?: string): string {
  return steerBlock({ kind: "edit" }, [{ ruleId, mode: "steer", message, location }]);
}

/** The rules a delivery names for `ruleIds`, each steered, and linked to its page on `dashboard` when one is given. */
export function steeredRules(ruleIds: readonly string[], dashboard?: string): SteerRule[] {
  return ruleIds.map(id => ({
    id,
    state: "steered",
    ...(dashboard === undefined ? {} : { url: `${dashboard}#rule/${encodeURIComponent(id)}` }),
  }));
}

type SteerShell = NonNullable<Parameters<typeof steerBlock>[2]>;

/**
 * The shell `host`'s agent runs the commands a steer names in, on the machine the test runs on: sh off Windows; on
 * Windows the shell its capability profile names (`windowsCommandShell`), PowerShell or else Git Bash.
 */
export function hostShell(host: RerailHost): SteerShell {
  if (process.platform !== "win32") return "posix";
  return ADAPTER_CAPABILITIES[host].windowsCommandShell === "powershell" ? "powershell" : "git-bash";
}

/** The `rerail feedback` command for `ruleId` as each shell takes the agent's line: an argument in sh, a pipe on Windows. */
const FEEDBACK_COMMANDS: Readonly<Record<SteerShell, (ruleId: string) => string>> = {
  "posix": ruleId => `rerail feedback ${ruleId} '<that line>'`,
  "git-bash": ruleId => `echo '<that line>' | rerail feedback ${ruleId} -`,
  "powershell": ruleId => `$OutputEncoding = [Text.UTF8Encoding]::new(); '<that line>' | rerail feedback ${ruleId} -`,
};

/**
 * The line that ends a steer about `ruleId` that the runtime sends `host`'s agent on this machine, as the agent reads
 * it: `Fix it` after an edit, `Follow it` for a tool call's guidance, then the feedback command in that agent's shell
 * (`hostShell`). It is written out here, not rendered by the steer code under test.
 */
export function steerLatitude(host: RerailHost, ruleId: string, verb: "Fix" | "Follow" = "Fix"): string {
  const shell = hostShell(host);
  const quotes = shell === "powershell" ? ", with each ' and ’ in the line written twice" : "";
  return `${verb} it, or tell the user in one line why you won't and run \`${FEEDBACK_COMMANDS[shell](ruleId)}\`${quotes}.`;
}
