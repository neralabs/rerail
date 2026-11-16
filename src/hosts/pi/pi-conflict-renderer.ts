import type { ExtensionAPI, Theme } from "@earendil-works/pi-coding-agent";
// Pi maps this import to its own copy (Pi's docs, "Available Imports"), so nothing of it is bundled and the components
// are the host's: its `Text` wraps each row to the transcript's width, which Pi requires of every rendered line.
import { Container, getCapabilities, Text } from "@earendil-works/pi-tui";
import { isConflictNotice, RULE_MESSAGE_TYPE, type ConflictNotice } from "../../engine/adapter-contract.js";
import { conflictLine } from "../../engine/code-rule/rerail-steer.js";
import { colourDepth, rerailMark } from "../../presentation/rerail-mark.js";

/** The theme colours the expanded rows are drawn with: Pi's own, so they read as part of the host. */
export type ConflictTheme = Pick<Theme, "fg">;

/** One row of the drawn line. A detail row is indented by `DETAIL_INDENT`, so it wraps under its own indentation. */
export interface ConflictRow {
  readonly detail: boolean;
  readonly text: string;
}

/** Expanded detail sits three columns in, under the line's text rather than under the glyph (as OMP draws it). */
const DETAIL_INDENT = 3;

const CONTROL = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu;
/** The note and the conflict's address are repository and dashboard text; nothing in them may move the cursor or open a link. */
const terminalText = (value: string): string => value.replace(CONTROL, "");

/** The conflict a Merge Assist note's details carry (`sendNativeDeliveries`); none when it is missing or malformed. */
function detailConflict(details: unknown): ConflictNotice | undefined {
  if (details === null || typeof details !== "object" || !("conflict" in details)) return undefined;
  return isConflictNotice(details.conflict) ? details.conflict : undefined;
}

/**
 * The person's line for the conflict a Merge Assist note surfaces, `◆ R E R A I L · your edit overlaps <label>
 * (<author>): <title>  ·  open` (`conflictLine`, the line Claude Code, Codex and OMP show), `open` linking its card
 * where Pi draws OSC 8 links and followed by its address where it does not. Expanded (the host's expand key), the link
 * is written out under it for copying and the note the coding agent read follows, dimmed.
 */
export function formatPiConflict(
  content: string,
  conflict: ConflictNotice,
  options: { expanded: boolean; hyperlinks: boolean; mark: string },
  theme: ConflictTheme,
): ConflictRow[] {
  const rows: ConflictRow[] = [{ detail: false, text: conflictLine(conflict, options.hyperlinks ? "hyperlink" : "plain", options.mark) }];
  if (!options.expanded) return rows;
  const { url } = conflict;
  if (url !== undefined && options.hyperlinks) {
    const target = terminalText(url);
    rows.push({ detail: true, text: `${theme.fg("dim", "open")} \u001b]8;;${target}\u0007${theme.fg("mdLink", target)}\u001b]8;;\u0007` });
  }
  const note = content.split("\n").filter(line => line.trim() !== "");
  return [...rows, ...note.map(line => ({ detail: true, text: theme.fg("dim", terminalText(line)) }))];
}

/**
 * Draws a `rerail.rule` message that carries a Merge Assist note's conflict as that conflict's line, in place of Pi's
 * default card, which shows the message type and the whole agent-facing note. Only the drawing changes: the coding
 * agent still receives the message's full `content`. A message with no conflict (a steer) is declined, so Pi draws it
 * as before.
 */
export function registerPiConflictRenderer(
  pi: Pick<ExtensionAPI, "registerMessageRenderer">,
  hyperlinks: () => boolean = () => getCapabilities().hyperlinks,
  mark: () => string = () => rerailMark(colourDepth()),
): void {
  pi.registerMessageRenderer(RULE_MESSAGE_TYPE, ({ content, details }, { expanded, outputPad }, theme) => {
    const conflict = detailConflict(details);
    if (conflict === undefined) return undefined;
    const text = typeof content === "string"
      ? content
      : content.flatMap(block => (block.type === "text" ? [block.text] : [])).join("\n");
    const drawn = new Container();
    for (const row of formatPiConflict(text, conflict, { expanded, hyperlinks: hyperlinks(), mark: mark() }, theme)) {
      drawn.addChild(new Text(row.text, outputPad + (row.detail ? DETAIL_INDENT : 0), 0));
    }
    return drawn;
  });
}
