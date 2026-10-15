import { describe, expect, it } from "vitest";
import { formatTerminalStatus, type SessionPresentation } from "../../src/presentation/session-presentation.js";
import { fixturesPath } from "../support/oracles.js";

const session = "http://127.0.0.1:4737/?sessionId=s1&host=claude-code#sessions";
const rules = "http://127.0.0.1:4737/#rules";

/** The footer's states (plan §3), as `readPresentation` builds them: the brand first, then the items with their links. */
const STATES: Record<string, Pick<SessionPresentation, "status" | "parts">> = {
  "on": { status: "on", parts: [{ text: "", url: session }, { text: "26 rules", url: rules }] },
  "on-steered": { status: "on", parts: [{ text: "", url: session }, { text: "26 rules", url: rules }, { text: "steered 3×", url: session }] },
  "not-on-here": { status: "not-on-here", parts: [{ text: "", url: null }, { text: "not on here", url: null }] },
  "not-running": { status: "not-running", parts: [{ text: "", url: null }, { text: "not running", url: null }] },
  "not-set-up": { status: "not-set-up", parts: [{ text: "", url: null }, { text: "not set up", url: rules }, { text: "rerail start", url: null }] },
};

const COLOURS: Record<string, Record<string, string>> = {
  "truecolor": { COLORTERM: "truecolor" },
  "256": { TERM: "xterm-256color" },
  "no-color": { COLORTERM: "truecolor", NO_COLOR: "1" },
};

describe("footer golden output", () => {
  for (const [state, presentation] of Object.entries(STATES)) {
    for (const [colour, env] of Object.entries(COLOURS)) {
      it(`${state} in ${colour}`, async () => {
        const raw = formatTerminalStatus({ ...presentation, text: "", url: null, presence: "compact", repositoryState: null, counts: null }, env);
        await expect(raw).toMatchFileSnapshot(fixturesPath("host", "session-enforcement", "footer", `${state}-${colour}.ansi`));
      });
    }
  }
});
