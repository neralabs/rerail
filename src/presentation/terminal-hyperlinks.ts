const TMUX_HYPERLINKS = { major: 3, minor: 4 };

/** Whether `version` (`3.4`, `3.5a`, `next-3.6`) is at least tmux 3.4, the first tmux to forward OSC 8. */
function tmuxForwardsLinks(version: string | undefined): boolean {
  const match = /(\d+)\.(\d+)/u.exec(version ?? "");
  if (match === null) return false;
  const major = Number(match[1]);
  const minor = Number(match[2]);
  return major > TMUX_HYPERLINKS.major || (major === TMUX_HYPERLINKS.major && minor >= TMUX_HYPERLINKS.minor);
}

/**
 * Variables a terminal that draws OSC 8 sets in every shell it starts, so they survive into tmux, which replaces
 * `TERM_PROGRAM` with its own. Orca names each of its terminals in `ORCA_TERMINAL_HANDLE`.
 */
const LINKING_TERMINAL_MARKERS = [
  "KITTY_WINDOW_ID", "GHOSTTY_RESOURCES_DIR", "WEZTERM_PANE", "ITERM_SESSION_ID", "VSCODE_PID", "ALACRITTY_WINDOW_ID", "WT_SESSION",
  "ORCA_TERMINAL_HANDLE",
] as const;

/**
 * `TERM_PROGRAM` of terminals that draw OSC 8. OMP's own list (pi-tui `KNOWN_TERMINALS`), plus Orca, whose links were
 * verified in Claude Code's status line (docs/in-session-surfaces.md).
 */
const LINKING_TERMINAL_PROGRAMS = new Set(["kitty", "ghostty", "wezterm", "iterm.app", "vscode", "alacritty", "orca"]);

function linkingTerminal(env: NodeJS.ProcessEnv): boolean {
  if (LINKING_TERMINAL_MARKERS.some(name => (env[name] ?? "") !== "")) return true;
  const term = env.TERM ?? "";
  return LINKING_TERMINAL_PROGRAMS.has(env.TERM_PROGRAM?.toLowerCase() ?? "") || term.includes("ghostty") || term.includes("kitty");
}

/**
 * Whether the terminal a hook's coding agent runs in draws OSC 8 links, judged from the environment the agent passes
 * its hooks. Mirrors OMP's `shouldEnableHyperlinksByDefault`: an override decides first (`RERAIL_NO_HYPERLINKS=1` or
 * OMP's `PI_NO_HYPERLINKS=1` off, which wins; `RERAIL_FORCE_HYPERLINKS=1` or `PI_FORCE_HYPERLINKS=1` on); GNU screen
 * never forwards links; otherwise only a terminal known to draw them does, through tmux from 3.4. A terminal it does
 * not know gets the link printed rather than an invisible one.
 */
export function terminalDrawsHyperlinks(env: NodeJS.ProcessEnv = process.env): boolean {
  if (env.RERAIL_NO_HYPERLINKS === "1" || env.PI_NO_HYPERLINKS === "1") return false;
  if (env.RERAIL_FORCE_HYPERLINKS === "1" || env.PI_FORCE_HYPERLINKS === "1") return true;
  if ((env.STY ?? "") !== "" || !linkingTerminal(env)) return false;
  if ((env.TMUX ?? "") !== "") return env.TERM_PROGRAM?.toLowerCase() === "tmux" && tmuxForwardsLinks(env.TERM_PROGRAM_VERSION);
  return true;
}
