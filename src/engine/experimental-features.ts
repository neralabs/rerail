/**
 * Experimental features: every feature in internal testing (docs/internal-testing.md), off unless turned on in the
 * process that runs it (the daemon for OMP, Claude Code, Codex and Cursor sessions, its API and dashboard, plus
 * Cursor's hook for its `stop` follow-up; the CLI process for a command of its own; the coding agent itself for OMP's
 * and Pi's tools and for a Pi session running Rerail in process). Their behaviour may change or go away.
 *
 * Each feature has its own variable, `RERAIL_EXPERIMENTAL_<FEATURE>`: `1` turns it on and `0` off, whatever the
 * blanket {@link EXPERIMENTAL_ALL} says. With the feature's variable unset (or any other value), `RERAIL_EXPERIMENTAL=1`
 * turns it on; otherwise it is off.
 */

/** The blanket switch: `1` turns on every experimental feature whose own variable is neither `1` nor `0`. */
export const EXPERIMENTAL_ALL = "RERAIL_EXPERIMENTAL";

/** Every experimental feature, by the suffix of its variable: what a person calls it, and whether that name is plural. */
export const EXPERIMENTAL_FEATURES = {
  MISSED_STEER_DIGEST: { name: "The missed-steer digest", plural: false },
  STEER_STRATEGIES: { name: "Steer strategies", plural: true },
  FIRST_REPOSITORY_AUDIT: { name: "The automatic first repository audit", plural: false },
  MERGE_ASSIST: { name: "Merge Assist", plural: false },
  REPOSITORY_SCAN: { name: "Full-repository scanning", plural: false },
  HEADLESS_SCAN: { name: "Headless scan", plural: false },
  CONVERSATION_RULES: { name: "Conversation rules", plural: true },
  SEMANTIC_TOOLS: { name: "Semantic tools", plural: true },
  BLOCK_MODE: { name: "Block mode", plural: false },
  ACTIVATION_PROFILES: { name: "Activation profiles", plural: true },
  EVENT_STREAM: { name: "The event stream", plural: false },
  REMOTE_SINK: { name: "The remote sink", plural: false },
} as const satisfies Record<string, { name: string; plural: boolean }>;

export type ExperimentalFeature = keyof typeof EXPERIMENTAL_FEATURES;

/** The variable that turns `feature` on or off: `RERAIL_EXPERIMENTAL_<feature>`. */
export function experimentVariable(feature: ExperimentalFeature): string {
  return `${EXPERIMENTAL_ALL}_${feature}`;
}

/** Whether `feature` is on in `env`: its own `1` or `0`, else the blanket switch. */
export function experimentEnabled(feature: ExperimentalFeature, env: NodeJS.ProcessEnv = process.env): boolean {
  const own = env[experimentVariable(feature)];
  if (own === "1" || own === "0") return own === "1";
  return env[EXPERIMENTAL_ALL] === "1";
}

/** The experimental features on in `env`, in {@link EXPERIMENTAL_FEATURES} order. */
export function enabledExperiments(env: NodeJS.ProcessEnv = process.env): ExperimentalFeature[] {
  return (Object.keys(EXPERIMENTAL_FEATURES) as ExperimentalFeature[]).filter(feature => experimentEnabled(feature, env));
}

/**
 * What a person reads when they reach `feature` while it is off: which variable turns it on, set where `where` says
 * (the process that runs it, such as "start the Rerail daemon with").
 */
export function experimentOffMessage(feature: ExperimentalFeature, where = "set"): string {
  const { name, plural } = EXPERIMENTAL_FEATURES[feature];
  return `${name} ${plural ? "are" : "is"} experimental and off: ${where} ${experimentVariable(feature)}=1 (or ${EXPERIMENTAL_ALL}=1 for every experimental feature) to use ${plural ? "them" : "it"}.`;
}

/** Reaching an experimental feature that is off. `code` is what the API and `--json` output name it. */
export class ExperimentOffError extends Error {
  readonly code = "experimental-off";
  readonly feature: ExperimentalFeature;

  constructor(feature: ExperimentalFeature, where?: string) {
    super(experimentOffMessage(feature, where));
    this.name = "ExperimentOffError";
    this.feature = feature;
  }
}

/** Throws {@link ExperimentOffError} unless `feature` is on in `env`. */
export function requireExperiment(feature: ExperimentalFeature, where?: string, env: NodeJS.ProcessEnv = process.env): void {
  if (!experimentEnabled(feature, env)) throw new ExperimentOffError(feature, where);
}

/**
 * Experimental: a code-rule steer whose check finishes after its step has moved on is held, and the agent
 * gets one digest of those findings, per rule with files and spans, when it finishes its run. Off, such a steer reaches
 * the agent at its next event or poll. `RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST`.
 */
export function missedSteerDigestEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentEnabled("MISSED_STEER_DIGEST", env);
}

/**
 * Experimental: a checkout's saved `steerStrategy` other than `message` (`rewind`, `subagent`) takes effect.
 * Off, every code-rule steer goes out as `message`, whatever the checkout saved. `RERAIL_EXPERIMENTAL_STEER_STRATEGIES`.
 */
export function steerStrategiesEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentEnabled("STEER_STRATEGIES", env);
}

/**
 * Experimental: the first session in a checkout starts one full-repository audit in log mode. Off, a session audits
 * only what it changes. `RERAIL_EXPERIMENTAL_FIRST_REPOSITORY_AUDIT`.
 */
export function firstRepositoryAuditEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentEnabled("FIRST_REPOSITORY_AUDIT", env);
}

/**
 * Experimental: Merge Assist. Off, nothing of it runs (no detection, scan, `gh`, conflict hook command, agent note,
 * person's line or footer segment), even for a repository a person turned it on for; its API operations answer
 * `404 experimental-off`, and `rerail merge-assist` refuses. On, a repository still has it only once a person turns it
 * on there. `RERAIL_EXPERIMENTAL_MERGE_ASSIST`.
 */
export function mergeAssistExperimentEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentEnabled("MERGE_ASSIST", env);
}

/**
 * Experimental: conversation rules, similarity (cosine) and command (tool-call) alike. Off, none is checked, whoever
 * added it and wherever it is stored (the library, a worktree's `.rerail/rules`, a rule pack); `rerail rules test` and
 * `rules add` refuse a conversation draft; and OMP and Pi register neither `conversation_rules` nor `/cosine-rules`.
 * `RERAIL_EXPERIMENTAL_CONVERSATION_RULES`.
 */
export function conversationRulesEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentEnabled("CONVERSATION_RULES", env);
}

/**
 * Experimental: OMP's and Pi's semantic tools, `rerail_mark`, `rerail_select`, `rerail_project` and `rerail_relate`.
 * Off, neither registers them. `RERAIL_EXPERIMENTAL_SEMANTIC_TOOLS`.
 */
export function semanticToolsEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentEnabled("SEMANTIC_TOOLS", env);
}

/**
 * Experimental: setting a rule's mode to `block` (which steers in this release). Off, a write that sets it is refused;
 * a rule already on Block keeps steering. `RERAIL_EXPERIMENTAL_BLOCK_MODE`.
 */
export function blockModeEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentEnabled("BLOCK_MODE", env);
}

/**
 * Experimental: the remote sink. Off, the daemon sends nothing to a remote added earlier, `rerail remote` refuses, and
 * a headless scan queues no report. `RERAIL_EXPERIMENTAL_REMOTE_SINK`.
 */
export function remoteSinkEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentEnabled("REMOTE_SINK", env);
}

/**
 * Experimental: activation profiles. Off, `rerail activate`, `activation request`, `profile` and `--profile` refuse,
 * and no stored profile restricts `rerail update`. `RERAIL_EXPERIMENTAL_ACTIVATION_PROFILES`.
 */
export function activationProfilesEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return experimentEnabled("ACTIVATION_PROFILES", env);
}
