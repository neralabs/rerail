/**
 * The footer's identity: the six letters `R E R A I L`, spaced one apart, each in its own colour (plan §3, D2).
 * Only the Claude Code status line and the OMP widget draw it; plain-text surfaces keep the word `Rerail`.
 */

/**
 * One colour per letter, pale end included. Source: `mark()` in the design `rerail-session-enforcement-v1.html`,
 * itself taken from `rerail_converged.py`.
 */
export const RERAIL_MARK_RAMP = ["#A5ADFB", "#B3B6F6", "#C1BFF2", "#D0C8ED", "#DED1E9", "#ECDAE4"] as const;

const LETTERS = ["R", "E", "R", "A", "I", "L"] as const;

/** The mark without colour, as the footer's plain `text` and every NO_COLOR terminal read it. */
export const RERAIL_MARK_TEXT = LETTERS.join(" ");

/** What the terminal can draw: 24-bit, the xterm 256-colour palette, or no colour at all. */
export type ColourDepth = "truecolor" | "256" | "none";

export type TerminalEnv = Readonly<Record<string, string | undefined>>;

/** `NO_COLOR` (non-empty) wins (no-color.org); `COLORTERM` alone advertises 24-bit; anything else gets 256 colours. */
export function colourDepth(env: TerminalEnv = process.env): ColourDepth {
  if (env.NO_COLOR) return "none";
  const colorterm = env.COLORTERM?.toLowerCase();
  return colorterm === "truecolor" || colorterm === "24bit" ? "truecolor" : "256";
}

function rgb(hex: string): [number, number, number] {
  const value = Number.parseInt(hex.slice(1), 16);
  return [value >> 16 & 0xff, value >> 8 & 0xff, value & 0xff];
}

const CUBE_LEVELS = [0, 95, 135, 175, 215, 255];
const distance = (a: readonly number[], b: readonly number[]): number => a.reduce((sum, v, i) => sum + (v - b[i]!) ** 2, 0);

/** The nearest xterm-256 index: the closer of the 6×6×6 cube (16–231) and the grey ramp (232–255). */
export function nearestXterm256(hex: string): number {
  const colour = rgb(hex);
  const cube = colour.map(v => CUBE_LEVELS.reduce((best, level, i) => Math.abs(level - v) < Math.abs(CUBE_LEVELS[best]! - v) ? i : best, 0));
  const cubeIndex = 16 + 36 * cube[0]! + 6 * cube[1]! + cube[2]!;
  const mean = (colour[0] + colour[1] + colour[2]) / 3;
  const grey = Math.min(23, Math.max(0, Math.round((mean - 8) / 10)));
  const greyLevel = 8 + 10 * grey;
  return distance(colour, cube.map(i => CUBE_LEVELS[i]!)) <= distance(colour, [greyLevel, greyLevel, greyLevel])
    ? cubeIndex : 232 + grey;
}

function foreground(hex: string, depth: ColourDepth): string {
  if (depth === "truecolor") return `\u001b[38;2;${rgb(hex).join(";")}m`;
  return `\u001b[38;5;${nearestXterm256(hex)}m`;
}

/** `R E R A I L`, each letter in its ramp colour; the spaces between stay uncoloured. */
export function rerailMark(depth: ColourDepth): string {
  if (depth === "none") return RERAIL_MARK_TEXT;
  return LETTERS.map((letter, i) => `${foreground(RERAIL_MARK_RAMP[i]!, depth)}${letter}\u001b[39m`).join(" ");
}

/** The state mark's colour (16-colour SGR): `●` green, `○` dim, `✕` red. */
const STATE_STYLE: Record<string, readonly [string, string]> = {
  "●": ["\u001b[32m", "\u001b[39m"],
  "○": ["\u001b[2m", "\u001b[22m"],
  "✕": ["\u001b[31m", "\u001b[39m"],
};

export function stateMark(mark: string, depth: ColourDepth): string {
  const style = STATE_STYLE[mark];
  return depth === "none" || style === undefined ? mark : `${style[0]}${mark}${style[1]}`;
}
