import type { ExtensionAPI, Theme } from "@oh-my-pi/pi-coding-agent";
import { isSteerRule, RULE_MESSAGE_TYPE, STEER_LINE_MESSAGE_TYPE, type SteerRule } from "../../engine/adapter-contract.js";
import { STEER_GLYPHS } from "../../engine/code-rule/rerail-steer.js";
import { colourDepth, rerailMark } from "../../presentation/rerail-mark.js";
import { terminalDrawsHyperlinks } from "../../presentation/terminal-hyperlinks.js";

/** The theme colours the steer is drawn with: OMP's own, so it reads as part of the host and not as a Rerail banner. */
export type SteerTheme = Pick<Theme, "fg">;

/** One row of the drawn steer. A detail row is padded by `DETAIL_INDENT`, so it wraps under its own indentation. */
export interface SteerRow {
  readonly detail: boolean;
  readonly text: string;
}

/** Expanded detail sits three columns in, under the rule id rather than under the glyph. */
const DETAIL_INDENT = 3;
/** The host's default card pads one column; the steer keeps that margin. */
const MARGIN = 1;

const CONTROL = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu;
/** Guidance and rule ids are repository text; nothing in them may move the cursor or open a link of its own. */
const terminalText = (value: string): string => value.replace(CONTROL, "");
/** BEL-terminated OSC 8, matching the host TUI's own link framing. */
const hyperlink = (url: string, text: string): string => `\u001b]8;;${terminalText(url)}\u0007${text}\u001b]8;;\u0007`;

/** The rules a steer message's details carry (`sendNativeDeliveries`, `steerLine`); none when they are missing or malformed. */
function detailRules(details: unknown): SteerRule[] {
  if (details === null || typeof details !== "object" || !("rules" in details) || !Array.isArray(details.rules)) return [];
  return details.rules.filter(isSteerRule);
}

/**
 * `◆ R E R A I L · <rule>  steered`, `steerHeadlines`' header: the glyph and state in the theme's accent, the separator
 * dimmed, and Rerail as its mark, the footer's coloured letters.
 */
function headerText({ id, state }: SteerRule, mark: string, theme: SteerTheme): string {
  return `${theme.fg("accent", STEER_GLYPHS[state])} ${mark}${theme.fg("dim", " · ")}${terminalText(id)}  ${theme.fg("accent", state)}`;
}

/** The `why?` after a headline: the word itself linked where the terminal draws links, else the word and its address. */
function whyText(url: string, hyperlinks: boolean, theme: SteerTheme): string {
  const separator = theme.fg("dim", "  ·  ");
  if (hyperlinks) return `${separator}${hyperlink(url, theme.fg("mdLink", "why?"))}`;
  return `${separator}${theme.fg("mdLink", "why?")}${theme.fg("dim", " → ")}${theme.fg("mdLinkUrl", terminalText(url))}`;
}

/**
 * The steer as the person sees it in OMP (design D4): one line per rule, `◆ R E R A I L · <rule>  steered  ·  why?`, the
 * same line Claude Code and Codex show, built from the rules the message carries. Expanded (Ctrl+O) adds under each
 * line where its rule comes from and its link written out, for copying (a terminal without links has it written on
 * the line already), then the block the coding agent read. A rule with no link (no dashboard was known) has no
 * `why?`: a link to nowhere would only mislead.
 */
export function formatOmpSteer(
  content: string,
  rules: readonly SteerRule[],
  options: { expanded: boolean; hyperlinks: boolean; mark: string },
  theme: SteerTheme,
): SteerRow[] {
  const rows = rules.flatMap(rule => {
    const { url } = rule;
    const headline: SteerRow = {
      detail: false,
      text: `${headerText(rule, options.mark, theme)}${url === undefined ? "" : whyText(url, options.hyperlinks, theme)}`,
    };
    if (!options.expanded) return [headline];
    const drawn: SteerRow[] = [headline];
    if (rule.origin !== undefined) drawn.push({ detail: true, text: theme.fg("dim", terminalText(rule.origin)) });
    if (url !== undefined && options.hyperlinks) {
      drawn.push({ detail: true, text: `${theme.fg("dim", "why?")} ${hyperlink(url, theme.fg("mdLink", terminalText(url)))}` });
    }
    return drawn;
  });
  if (rows.length === 0 || !options.expanded) return rows;
  const block = content.split("\n").filter(line => line.trim() !== "");
  return [...rows, ...block.map(line => ({ detail: true, text: theme.fg("dim", terminalText(line)) }))];
}

type HostComponents = Pick<ExtensionAPI["pi"], "Container" | "Text">;

/**
 * Draws `rerail.rule` messages, and the `rerail.steer-line` the person sees for steers that rode a tool result, as the
 * steer's one line per rule instead of OMP's default card, which repeats the whole model-facing block under the message
 * type. The model still receives a `rerail.rule` message's full `content`. The components are the host's own (`pi.pi`),
 * never bundled copies; without them, or with no rule in the message's details, the renderer declines and OMP's default
 * card shows the content.
 */
export function registerOmpSteerRenderer(
  pi: Pick<ExtensionAPI, "registerMessageRenderer"> & { pi?: Partial<HostComponents> },
  hyperlinks: () => boolean = terminalDrawsHyperlinks,
  mark: () => string = () => rerailMark(colourDepth()),
): void {
  for (const customType of [RULE_MESSAGE_TYPE, STEER_LINE_MESSAGE_TYPE]) {
    pi.registerMessageRenderer(customType, ({ content, details }, { expanded }, theme) => {
      const { Container, Text } = pi.pi ?? {};
      if (Container === undefined || Text === undefined) return undefined;
      const text = typeof content === "string"
        ? content
        : content.flatMap(block => (block.type === "text" ? [block.text] : [])).join("\n");
      const rows = formatOmpSteer(text, detailRules(details), { expanded, hyperlinks: hyperlinks(), mark: mark() }, theme);
      if (rows.length === 0) return undefined;
      const steer = new Container();
      for (const row of rows) steer.addChild(new Text(row.text, MARGIN + (row.detail ? DETAIL_INDENT : 0), 0));
      return steer;
    });
  }
}
