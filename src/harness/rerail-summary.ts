import { rerailCommandList } from "./rerail-commands.js";
import type { CommandSigil } from "../engine/adapter-contract.js";
import type { RepositoryStartState } from "../engine/catalog/repository-model.js";
import type { SessionPresentation } from "../presentation/session-presentation.js";

export interface RerailSummaryInput {
  /** This session's presentation, read on demand; null where the coding agent cannot name its session (Cursor). */
  presentation: SessionPresentation | null;
  /** The repository's state when there is no session presentation; ignored otherwise. */
  repositoryState: RepositoryStartState | null;
  /** Rules on in this repository, for the no-session form. */
  ruleCount: number | null;
  /** Rules and packs on in this repository that run in Log waiting for the person's approval, for the no-session form. */
  awaitingApproval: number | null;
  /** The dashboard's rules page, when a daemon answers. */
  rulesUrl: string | null;
  repositoryName: string;
  sigil: CommandSigil;
}

const plural = (count: number, noun: string): string => `${count} ${noun}${count === 1 ? "" : "s"}`;

const NOT_RUNNING: readonly string[] = ["✕ Rerail is not running.", "  Start it with `rerail daemon start` in a terminal."];

const notOnLines = ({ repositoryName, sigil }: RerailSummaryInput): string[] =>
  [`○ Rerail is not on in ${repositoryName}.`, `  Start it with ${sigil}rerail-start.`];

const notSetUpLines = ({ repositoryName, sigil }: RerailSummaryInput): string[] =>
  [`○ Rerail is on in ${repositoryName}, which is not set up yet: no rules are checked.`, `  Set it up with ${sigil}rerail-start (rerail start).`];

const onLine = ({ repositoryName }: RerailSummaryInput): string => `● Rerail is on in this repo · ${repositoryName}`;

/**
 * `N to approve, running in Log`, linked to the dashboard, whose Overview lists them, or to `rerail rules approve`,
 * which lists them too; nothing when none wait.
 */
function awaitingLines(awaiting: number | null, rulesUrl: string | null): string[] {
  if (awaiting === null || awaiting === 0) return [];
  return [`  ${awaiting} to approve, running in Log → ${rulesUrl ?? "rerail rules approve"}`];
}

/**
 * The headline without a session (Cursor, or OMP/Pi where none is attached): from the repository's state. With no
 * session to show, the link is to the rules this repository runs (design N8, Cursor).
 */
function repositoryHeadline(input: RerailSummaryInput): readonly string[] {
  const { repositoryState, ruleCount, rulesUrl } = input;
  if (repositoryState === null) return NOT_RUNNING;
  if (repositoryState === "not-set-up") return notSetUpLines(input);
  if (repositoryState === "stopped") return notOnLines(input);
  const lines = [onLine(input)];
  if (ruleCount !== null) lines.push(`  ${plural(ruleCount, "rule")}`);
  lines.push(...awaitingLines(input.awaitingApproval, rulesUrl));
  if (rulesUrl !== null) lines.push(`  Manage rules → ${rulesUrl}`);
  return lines;
}

function steeringText(steered: number | null): string {
  if (steered === null) return "steering unavailable";
  if (steered === 0) return "nothing steered yet in this session";
  return `steered ${steered}× in this session`;
}

function headline(input: RerailSummaryInput): readonly string[] {
  const { presentation } = input;
  if (presentation === null) return repositoryHeadline(input);
  switch (presentation.status) {
    case "not-on-here":
      return notOnLines(input);
    case "not-set-up":
      return notSetUpLines(input);
    case "not-running":
      return NOT_RUNNING;
    case "starting":
      return [onLine(input), "  This session is not recorded yet; ask again after your next message."];
    case "unavailable":
      return [onLine(input), "  This session could not be read."];
    case "on": {
      const { rules, steered, awaitingApproval } = presentation.counts ?? { rules: null, steered: null, awaitingApproval: null };
      const coverage = rules === null ? "rules unavailable" : plural(rules, "rule");
      const lines = [onLine(input), `  ${coverage} · ${steeringText(steered)}`, ...awaitingLines(awaitingApproval, input.rulesUrl)];
      if (presentation.url !== null) lines.push(`  See what it logged → ${presentation.url}`);
      return lines;
    }
  }
}

/** `/rerail` (design L1): what Rerail is doing here, then every command, one per line. */
export function renderRerailSummary(input: RerailSummaryInput): string {
  return [...headline(input), "", rerailCommandList(input.sigil)].join("\n");
}
