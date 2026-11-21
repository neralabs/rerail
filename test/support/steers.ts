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

/**
 * How a steer tells the agent to record its line for `ruleId` in each shell: as an argument in sh, through a pipe in Git
 * Bash, and in PowerShell as three lines, the line in a here-string piped in as UTF-8.
 */
const FEEDBACK_ACTIONS: Readonly<Record<SteerShell, (ruleId: string) => string>> = {
  "posix": ruleId => `run \`rerail feedback ${ruleId} '<that line>'\``,
  "git-bash": ruleId => `run \`echo '<that line>' | rerail feedback ${ruleId} -\``,
  "powershell": ruleId =>
    `record that line with three PowerShell lines: \`$OutputEncoding = [Text.UTF8Encoding]::new(); @'\`, a space and the line, then \`'@ | rerail feedback ${ruleId} -\``,
};

/**
 * The line that ends a steer about `ruleId` in `shell`, as the agent reads it: `Fix it` after an edit, `Follow it` for
 * a tool call's guidance, then how to record the agent's line in that shell. It is written out here, not rendered by
 * the steer code under test; test/engine/code-rule/rerail-steer.test.ts holds the two equal.
 */
export function shellLatitude(shell: SteerShell, ruleId: string, verb: "Fix" | "Follow" = "Fix"): string {
  return `${verb} it, or tell the user in one line why you won't and ${FEEDBACK_ACTIONS[shell](ruleId)}.`;
}

/** {@link shellLatitude} for the steer the runtime sends `host`'s agent on this machine, in that agent's shell (`hostShell`). */
export function steerLatitude(host: RerailHost, ruleId: string, verb: "Fix" | "Follow" = "Fix"): string {
  return shellLatitude(hostShell(host), ruleId, verb);
}
