import type { ExtensionAPI, Theme } from "@oh-my-pi/pi-coding-agent";
import { RULE_MESSAGE_TYPE } from "../../engine/adapter-contract.js";
import { steerHeadlines, type SteerHeadline } from "../../engine/code-rule/rerail-steer.js";
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

/** `steerHeadlines`' header, `◆ Rerail · <rule>  steered`, split so each part takes its own colour. */
const HEADER = /^(\S+) Rerail · (.+) {2}(\S+)$/u;
const WHY_LINE = /^Why\? → /u;

/** A steer block as drawn: its headline, and the lines between the header and the `Why?` link. */
interface DrawnBlock {
  readonly headline: SteerHeadline;
  readonly detail: readonly string[];
}

/** Every steer block in `content`, in order: the headline `steerHeadlines` parsed, with that block's own lines. */
function steerBlocks(content: string): DrawnBlock[] {
  const headlines = steerHeadlines(content);
  const blocks: Array<{ headline: SteerHeadline; detail: string[] }> = [];
  let open: { headline: SteerHeadline; detail: string[] } | undefined;
  for (const line of content.split("\n")) {
    const next = headlines[blocks.length];
    if (next !== undefined && line === next.header) {
      open = { headline: next, detail: [] };
      blocks.push(open);
    } else if (WHY_LINE.test(line)) {
      open = undefined;
    } else if (open !== undefined && line.trim() !== "") {
      open.detail.push(line);
    }
  }
  return blocks;
}

/** `◆ Rerail · <rule>  steered`: the glyph and state in the theme's accent, the separator dimmed. */
function headerText(header: string, theme: SteerTheme): string {
  const parts = HEADER.exec(terminalText(header));
  if (parts === null) return terminalText(header);
  const [, glyph, ruleId, state] = parts;
  return `${theme.fg("accent", glyph!)} Rerail${theme.fg("dim", " · ")}${ruleId}  ${theme.fg("accent", state!)}`;
}

/** The `why` after a headline: the word itself linked where the terminal draws links, else the word and its address. */
function whyText(url: string, hyperlinks: boolean, theme: SteerTheme): string {
  const separator = theme.fg("dim", "  ·  ");
  if (hyperlinks) return `${separator}${hyperlink(url, theme.fg("mdLink", "why"))}`;
  return `${separator}${theme.fg("mdLink", "why")}${theme.fg("dim", " → ")}${theme.fg("mdLinkUrl", terminalText(url))}`;
}

/**
 * The steer as the person sees it in OMP (design D4): one line per rule, `◆ Rerail · <rule>  steered  ·  why`, the
 * same line Claude Code and Codex show. Expanded (Ctrl+O) adds each block's own lines beneath its headline, where the
 * rule comes from, its mode and the guidance, then the link written out, for copying; a terminal without links has
 * it written on the headline already. A headline whose link is still relative (no dashboard was known) has no `why`:
 * a link to nowhere would only mislead.
 */
export function formatOmpSteer(content: string, options: { expanded: boolean; hyperlinks: boolean }, theme: SteerTheme): SteerRow[] {
  return steerBlocks(content).flatMap(({ headline, detail }) => {
    const { url } = headline;
    const rows: SteerRow[] = [{
      detail: false,
      text: `${headerText(headline.header, theme)}${url === undefined ? "" : whyText(url, options.hyperlinks, theme)}`,
    }];
    if (!options.expanded) return rows;
    rows.push(...detail.map(line => ({ detail: true, text: theme.fg("dim", terminalText(line)) })));
    if (url !== undefined && options.hyperlinks) {
      rows.push({ detail: true, text: `${theme.fg("dim", "why:")} ${hyperlink(url, theme.fg("mdLink", terminalText(url)))}` });
    }
    return rows;
  });
}

type HostComponents = Pick<ExtensionAPI["pi"], "Container" | "Text">;

/**
 * Draws `rerail.rule` messages as the steer's one line per rule instead of OMP's default card, which repeats the
 * whole model-facing block under a `rerail.rule` label. The model still receives the message's full `content`. The
 * components are the host's own (`pi.pi`), never bundled copies; without them, or with no Rerail headline in the
 * message, the renderer declines and OMP's default card shows the content.
 */
export function registerOmpSteerRenderer(
  pi: Pick<ExtensionAPI, "registerMessageRenderer"> & { pi?: Partial<HostComponents> },
  hyperlinks: () => boolean = terminalDrawsHyperlinks,
): void {
  pi.registerMessageRenderer(RULE_MESSAGE_TYPE, ({ content }, { expanded }, theme) => {
    const { Container, Text } = pi.pi ?? {};
    if (Container === undefined || Text === undefined) return undefined;
    const text = typeof content === "string"
      ? content
      : content.flatMap(block => (block.type === "text" ? [block.text] : [])).join("\n");
    const rows = formatOmpSteer(text, { expanded, hyperlinks: hyperlinks() }, theme);
    if (rows.length === 0) return undefined;
    const steer = new Container();
    for (const row of rows) steer.addChild(new Text(row.text, MARGIN + (row.detail ? DETAIL_INDENT : 0), 0));
    return steer;
  });
}
