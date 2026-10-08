import { rerailCommandList, type CommandSigil } from "./rerail-commands.js";
import type { RepositoryStartState } from "./repository-start.js";
import type { SessionPresentation } from "./session-presentation.js";

export interface RerailSummaryInput {
  /** This session's presentation, read on demand; null where the coding agent cannot name its session (Cursor). */
  presentation: SessionPresentation | null;
  /** The repository's state when there is no session presentation; ignored otherwise. */
  repositoryState: RepositoryStartState | null;
  /** Rules on in this repository, for the no-session form. */
  ruleCount: number | null;
  /** The dashboard's rules page, when a daemon answers. */
  rulesUrl: string | null;
  repositoryName: string;
  sigil: CommandSigil;
}

const plural = (count: number, noun: string): string => `${count} ${noun}${count === 1 ? "" : "s"}`;

const NOT_RUNNING: readonly string[] = ["✕ Rerail is not running.", "  Start it with `rerail daemon start` in a terminal."];

const notOnLines = ({ repositoryName, sigil }: RerailSummaryInput): string[] =>
  [`○ Rerail is not on in ${repositoryName}.`, `  Start it with ${sigil}rerail-start.`];

const onLine = ({ repositoryName }: RerailSummaryInput): string => `● Rerail is on in this repo · ${repositoryName}`;

/** The headline without a session (Cursor, or OMP/Pi where none is attached): from the repository's state. */
function repositoryHeadline(input: RerailSummaryInput): readonly string[] {
  const { repositoryState, ruleCount, rulesUrl } = input;
  if (repositoryState === null) return NOT_RUNNING;
  if (repositoryState !== "started") return notOnLines(input);
  const lines = [onLine(input)];
  if (ruleCount !== null) lines.push(`  ${plural(ruleCount, "rule")} on`);
  if (rulesUrl !== null) lines.push(`  Dashboard → ${rulesUrl}`);
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
    case "not-running":
      return NOT_RUNNING;
    case "unavailable":
      return [onLine(input), "  This session is not recorded yet; ask again after your next message."];
    case "on": {
      const { rules, steered } = presentation.counts ?? { rules: null, steered: null };
      const coverage = rules === null ? "rules unavailable" : plural(rules, "rule");
      const lines = [onLine(input), `  ${coverage} · ${steeringText(steered)}`];
      if (presentation.url !== null) lines.push(`  See what it logged → ${presentation.url}`);
      return lines;
    }
  }
}

/** `/rerail` (design L1): what Rerail is doing here, then every command, one per line. */
export function renderRerailSummary(input: RerailSummaryInput): string {
  return [...headline(input), "", rerailCommandList(input.sigil)].join("\n");
}
