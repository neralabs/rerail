import { describe, expect, it } from "vitest";
import {
  CONFIRM_PACKS_ANSWERS,
  NO_REPO_ANSWERS,
  READ_REPO_ANSWERS,
  renderCannotInstall,
  renderInstalled,
  renderNoRepoQuestion,
  renderNodeTooOld,
  renderPackReview,
  renderPickedMyself,
  renderReadRepoQuestion,
  renderRecommendedPacks,
  renderSkipped,
  renderStarted,
  renderSummary,
  renderTips,
  shellQuote,
  type CodingAgent,
  type DashboardLink,
  type RecommendedPacksInput,
  type SetupLane,
  type SummaryInput,
} from "../../src/harness/setup-screens.js";

const ANSI = /\u001b\[/;
const stripAnsi = (text: string): string => text.replace(/\u001b\[[0-9;]*m/g, "");

const CLAUDE: CodingAgent = { id: "claude", name: "Claude Code" };
const CURSOR: CodingAgent = { id: "cursor", name: "Cursor" };
const NOT_FOUND: CodingAgent[] = [
  { id: "codex", name: "Codex" },
  { id: "omp", name: "OMP" },
  { id: "pi", name: "Pi" },
];
const OPENED: DashboardLink = { url: "http://127.0.0.1:7777/#rules", browserOpened: true };
const NOT_OPENED: DashboardLink = { url: "http://127.0.0.1:7777/#rules", browserOpened: false, noBrowserReason: "this looks like a remote session" };

const PACKS: RecommendedPacksInput = {
  repoName: "acme-web",
  repoPath: "/Users/me/code/acme-web",
  packs: [
    { name: "TSLint", ruleCount: 74, reason: "matches the typescript dependency in package.json and *.ts files" },
    { name: "acme-api", ruleCount: 92, reason: "matches the acme_api package" },
  ],
  agentsFound: [CLAUDE, CURSOR],
  agentsNotFound: NOT_FOUND,
};

const SUMMARY: SummaryInput = {
  repoName: "acme-web",
  packNames: ["TSLint", "acme-api"],
  ruleCount: 166,
  connected: [CLAUDE, CURSOR],
  dashboard: OPENED,
};

const LANES: readonly SetupLane[] = ["terminal", "agent"];

/** Every screen, rendered in one lane, so lane-wide properties can be checked once. */
function everyScreen(lane: SetupLane): Record<string, string> {
  return {
    T2: renderInstalled({ version: "0.3.0" }, lane),
    T3: renderReadRepoQuestion({ repoName: "acme-web", repoPath: "/Users/me/code/acme-web" }, lane),
    T4: renderRecommendedPacks(PACKS, lane),
    WR: renderPackReview({ ...PACKS, review: { url: "http://127.0.0.1:7777/#review", browserOpened: true } }, lane),
    T5: renderStarted({ repoName: "acme-web", packCount: 5, connected: [CLAUDE, CURSOR] }, lane),
    T6: renderSummary(SUMMARY, lane),
    C6b: renderTips(lane),
    E1: renderCannotInstall({ installCommand: "npx rerail" }, lane),
    E2: renderNodeTooOld({ found: "20.11.1", requiredMajor: 24 }, lane),
    E3: renderNoRepoQuestion({}, lane),
    E4: renderPickedMyself({ repoName: "acme-web", connected: [CLAUDE, CURSOR], dashboard: OPENED }, lane),
    E5: renderSkipped({ connected: [CLAUDE, CURSOR] }, lane),
    E6: renderSummary({ ...SUMMARY, dashboard: NOT_OPENED }, lane),
  };
}

describe("both lanes", () => {
  it("the agent lane is plain text and the terminal lane is styled", () => {
    for (const [screen, text] of Object.entries(everyScreen("agent"))) expect(text, screen).not.toMatch(ANSI);
    for (const screen of ["T2", "T3", "T4", "T6", "E2", "E3"]) expect(everyScreen("terminal")[screen], screen).toMatch(ANSI);
  });

  it("no screen mentions /rerail-test, which is not in v1", () => {
    for (const lane of LANES) {
      for (const [screen, text] of Object.entries(everyScreen(lane))) expect(text, `${lane} ${screen}`).not.toContain("rerail-test");
    }
  });

  it("strips control characters from data before printing it", () => {
    const text = renderStarted({ repoName: "acme\u001b]8;;evil\u0007-web", packCount: 1, connected: [CLAUDE] }, "agent");
    expect(text).toContain("Rerail is on in acme]8;;evil-web.");
    expect(text).not.toMatch(/[\u001b\u0007]/);
  });
});

describe("T2 installed", () => {
  it("says the version is installed and nothing is on yet", () => {
    expect(stripAnsi(renderInstalled({ version: "0.3.0" }, "terminal"))).toContain(">^ᴥ^<   rerail 0.3.0 installed  nothing is on yet");
    expect(renderInstalled({ version: "0.3.0" }, "agent")).toContain("Rerail is installed. Nothing is on yet.");
  });
});

describe("T3 / C3 ask before reading", () => {
  const input = { repoName: "acme-web", repoPath: "/Users/me/code/acme-web" };

  it.each(LANES)("names the repo, says how it reads, and offers three answers (%s)", lane => {
    const text = stripAnsi(renderReadRepoQuestion(input, lane));
    expect(text).toContain("You’re working in acme-web. Can Rerail read it to pick the rule packs that look right?");
    expect(text).toContain("It matches the files in this repo against the rule packs that come with Rerail.");
    expect(text).toContain("It reads on this computer only. Nothing leaves it. Nothing is turned on yet.");
    expect(text).toContain("1. Yes, read acme-web");
    expect(text).toContain("2. Use a different folder…  for example “~/code/landing-page”");
    expect(text).toContain("3. I’ll pick rule packs myself  Rerail won’t read the repo");
  });

  it("puts the terminal cursor on the first answer", () => {
    expect(stripAnsi(renderReadRepoQuestion(input, "terminal"))).toContain("❯ 1. Yes, read acme-web");
  });

  it("tells the coding agent to ask, not answer, and how to call set-up again for each answer", () => {
    const text = renderReadRepoQuestion(input, "agent");
    expect(READ_REPO_ANSWERS).toEqual(["yes", "other-folder", "pick-myself"]);
    expect(text).toContain("Ask them this, as written, and wait for their reply");
    expect(text).toContain("Do not choose for them.");
    expect(text).toContain("1. rerail setup --answer yes --repo /Users/me/code/acme-web");
    expect(text).toContain("2. rerail setup --repo <the folder they name>");
    expect(text).toContain("3. rerail setup --answer pick-myself --repo /Users/me/code/acme-web");
    expect(text.indexOf("1. Yes, read acme-web")).toBeLessThan(text.indexOf("1. rerail setup"));
  });

  it("quotes a repo path the shell would split, and uses the given set-up command", () => {
    const text = renderReadRepoQuestion({ repoName: "it's mine", repoPath: "/tmp/it's mine", setupCommand: "npx rerail setup" }, "agent");
    expect(text).toContain("1. npx rerail setup --answer yes --repo '/tmp/it'\\''s mine'");
  });

  it("quotes a Windows path with double quotes, as cmd.exe runs it, and leaves a plain word alone", () => {
    expect(shellQuote(String.raw`C:\Users\Alice O'Neil\.rerail\bin\rerail.cmd`, "win32")).toBe(String.raw`"C:\Users\Alice O'Neil\.rerail\bin\rerail.cmd"`);
    expect(shellQuote("--answer", "win32")).toBe("--answer");
    expect(shellQuote(String.raw`C:\Users\Alice O'Neil\.rerail\bin\rerail.cmd`, "linux")).toBe(String.raw`'C:\Users\Alice O'\''Neil\.rerail\bin\rerail.cmd'`);
  });
});

describe("T4 / C4 recommended rule packs", () => {
  it.each(LANES)("lists each rule pack first, then its rule count, then why (%s)", lane => {
    const text = stripAnsi(renderRecommendedPacks(PACKS, lane));
    expect(text).toContain("Of the rule packs that come with Rerail, 2 look right for acme-web.");
    expect(text).toMatch(/Rule pack\s+Rules\s+Why/);
    expect(text).toMatch(/TSLint\s+74\s+matches the typescript dependency in package\.json and \*\.ts files/);
    expect(text).toMatch(/acme-api\s+92\s+matches the acme_api package/);
    expect(text).toContain("Connected to   Claude Code, Cursor  not found: Codex, OMP, Pi");
    expect(text).toContain("Every rule starts in Steer: your coding agent is told as it works, and fixes it.");
    expect(text).toContain("More rule packs are in the dashboard, off until you turn them on.");
    expect(text).toContain("Start Rerail in acme-web with these 2 rule packs?");
    expect(text).toContain("Nothing is on until you say yes.");
    expect(text).toContain("1. Yes, start Rerail");
    expect(text).toContain("2. Review them first  opens the dashboard");
    expect(text).toContain("3. Type something else…  only some coding agents, by id");
    expect(text).toContain("4. I’ll pick rule packs myself");
    expect(text).not.toMatch(/not covered|not relevant/i);
  });

  it("says how many rules start in Log when a rule pack starts some there, and leaves the line out with no packs", () => {
    const text = renderRecommendedPacks({ ...PACKS, packs: [{ ...PACKS.packs[0]!, logRuleCount: 3 }] }, "agent");
    expect(text).toContain("Rules start in Steer: your coding agent is told as it works, and fixes it. 3 start in Log, recorded only.");
    expect(renderRecommendedPacks({ ...PACKS, packs: [] }, "agent")).not.toContain("starts in Steer");
  });

  it("aligns the table columns for long rule pack names", () => {
    const text = renderRecommendedPacks({ ...PACKS, packs: [{ name: "A very long rule pack name", ruleCount: 12, reason: "the reason" }] }, "agent");
    const header = text.split("\n").find(line => line.includes("Rule pack")) ?? "";
    const row = text.split("\n").find(line => line.includes("A very long")) ?? "";
    expect(row.indexOf("12")).toBe(header.indexOf("Rules"));
    expect(row.indexOf("the reason")).toBe(header.indexOf("Why"));
  });

  it("lists the rules the lint config starts off or configures, then what it did not read, counting past eight lines", () => {
    const settings = Array.from({ length: 8 }, (_, index) => ({ ruleId: `eslint-rule-${index}`, folder: index === 0 ? "tests" : "", on: false as const, config: null, because: `.eslintrc.json: "rule-${index}": "off"` }));
    const text = renderRecommendedPacks({
      ...PACKS,
      lintConfig: {
        settings: [{ ruleId: "typescript-eslint-consistent-type-definitions", folder: "", on: null, config: { option: "type" }, because: ".eslintrc.json: \"x\": [\"error\",\"type\"]" }, ...settings],
        notes: ["eslint.config.ts is not read: Rerail never runs your config."],
      },
    }, "agent");
    expect(text).toContain("  From your lint config (read as data, never run):\n    typescript-eslint-consistent-type-definitions starts with option = \"type\"  .eslintrc.json: \"x\": [\"error\",\"type\"]\n    eslint-rule-0 starts off in tests/  .eslintrc.json: \"rule-0\": \"off\"\n    eslint-rule-1 starts off  .eslintrc.json: \"rule-1\": \"off\"");
    expect(text).toContain("    eslint-rule-5 starts off");
    expect(text).not.toContain("eslint-rule-6");
    expect(text).toContain("    …and 3 more.");
    expect(renderRecommendedPacks({ ...PACKS, lintConfig: { settings: [], notes: [] } }, "agent")).not.toContain("lint config");
  });

  it("echoes the yes in the terminal only", () => {
    expect(stripAnsi(renderRecommendedPacks(PACKS, "terminal"))).toContain("● You said yes. Rerail read acme-web.");
    expect(renderRecommendedPacks(PACKS, "agent")).not.toContain("You said yes");
  });

  it("speaks of one rule pack in the singular", () => {
    const text = renderRecommendedPacks({ ...PACKS, packs: PACKS.packs.slice(0, 1) }, "agent");
    expect(text).toContain("Of the rule packs that come with Rerail, 1 looks right for acme-web.");
    expect(text).toContain("Start Rerail in acme-web with this rule pack?");
  });

  it("leaves out the not-found part when every coding agent was found", () => {
    expect(renderRecommendedPacks({ ...PACKS, agentsNotFound: [] }, "agent")).not.toContain("not found");
  });

  it("gives the coding agent one command per answer, carrying the repo", () => {
    const text = renderRecommendedPacks(PACKS, "agent");
    expect(CONFIRM_PACKS_ANSWERS).toEqual(["start", "review", "other", "pick-myself"]);
    expect(text).toContain("1. rerail setup --answer start --repo /Users/me/code/acme-web");
    expect(text).toContain("2. rerail setup --answer review --repo /Users/me/code/acme-web");
    expect(text).toContain("--answer start --repo /Users/me/code/acme-web --agents <ids>");
    expect(text).toContain("claude (Claude Code), cursor (Cursor)");
    expect(text).toContain("4. rerail setup --answer pick-myself --repo /Users/me/code/acme-web");
  });
});

describe("WR review, then confirm back", () => {
  it.each(LANES)("gives the review link and asks to confirm back here, without a stale count (%s)", lane => {
    const text = stripAnsi(renderPackReview({ ...PACKS, review: { url: "http://127.0.0.1:7777/#review", browserOpened: false } }, lane));
    expect(text).toContain("then come back here to start Rerail.");
    expect(text).toContain("http://127.0.0.1:7777/#review");
    expect(text).toContain("Couldn’t open a browser from here.");
    expect(text).toContain("Start Rerail in acme-web with the rule packs you kept on?");
    expect(text).not.toContain("these 2 rule packs");
    expect(text).toContain("1. Yes, start Rerail");
  });
});

describe("T5 / C5 started", () => {
  it("says Rerail is on in this repo and names the connected coding agents", () => {
    const terminal = stripAnsi(renderStarted({ repoName: "acme-web", packCount: 2, connected: [CLAUDE, CURSOR] }, "terminal"));
    expect(terminal).toContain("● Confirmed: 2 rule packs · Claude Code and Cursor");
    expect(terminal).toContain("Rerail is on in acme-web.  connected to Claude Code and Cursor");
    expect(terminal).toContain("Enter to see the summary.");
    const agent = renderStarted({ repoName: "acme-web", packCount: 2, connected: [CLAUDE, CURSOR] }, "agent");
    expect(agent).toContain("Rerail is on in acme-web. Connected to Claude Code and Cursor.");
    expect(agent).not.toContain("Enter");
  });

  it("lists three coding agents with one 'and'", () => {
    const text = renderStarted({ repoName: "r", packCount: 1, connected: [CLAUDE, CURSOR, { id: "codex", name: "Codex" }] }, "agent");
    expect(text).toContain("Connected to Claude Code, Cursor and Codex.");
  });
});

describe("T6 / C6 summary", () => {
  it.each(LANES)("has every decided line, ending on the link (%s)", lane => {
    const text = stripAnsi(renderSummary(SUMMARY, lane));
    expect(text).toContain("● Rerail is on in acme-web. 2 rule packs are on, with 166 rules.");
    expect(text).toContain("Rule packs     TSLint · acme-api");
    expect(text).toContain("Your own       /rerail-create  create a rule in your own words, in a session");
    expect(text).toContain("Other repos    Rerail has not been started there  type /rerail-start there to set one up");
    expect(text).toContain("Connected      Claude Code and Cursor");
    expect(text).toContain("Next           Open a new session when you’re ready.  Rerail starts there");
    expect(text).toContain("Manage rules   http://127.0.0.1:7777/#rules  opening in your browser");
    expect(text).not.toMatch(/close this/i);
  });

  it("puts the tips below the divider, after the link, in the terminal", () => {
    const text = stripAnsi(renderSummary(SUMMARY, "terminal"));
    const link = text.indexOf("Manage rules");
    const divider = text.indexOf("── Good to know · optional");
    expect(divider).toBeGreaterThan(link);
    expect(text.indexOf("Three things can happen")).toBeGreaterThan(divider);
    expect(text.indexOf("Rerail never edits your instruction files.")).toBeGreaterThan(divider);
  });

  it("dims every tip line in the terminal", () => {
    const tips = renderTips("terminal").trimEnd().split("\n");
    for (const line of tips) expect(line).toMatch(/^\u001b\[2m.*\u001b\[22m$/);
  });

  it("offers the tips in the agent lane and keeps them for when the person asks", () => {
    const text = renderSummary(SUMMARY, "agent");
    const offer = text.indexOf("Want a few tips for your first session? Just ask.");
    expect(offer).toBeGreaterThan(text.indexOf("Manage rules"));
    expect(text.indexOf("Only if they ask for tips")).toBeGreaterThan(offer);
    expect(text.indexOf("A few things for your first session.")).toBeGreaterThan(offer);
    expect(text).not.toContain("── Good to know");
  });

  it("names other repos where Rerail is already on, instead of saying it is on nowhere else", () => {
    const text = renderSummary({ ...SUMMARY, otherStartedRepos: ["landing-page"] }, "agent");
    expect(text).toContain("Rerail is also on in landing-page");
    expect(text).not.toContain("has not been started there");
  });

  it("speaks of one rule pack and one rule in the singular", () => {
    expect(renderSummary({ ...SUMMARY, packNames: ["TSLint"], ruleCount: 1 }, "agent")).toContain("1 rule pack is on, with 1 rule.");
  });

  it("says no rule packs are on, not zero of them, where none fit the repo", () => {
    const text = renderSummary({ ...SUMMARY, packNames: [], ruleCount: 0 }, "agent");
    expect(text).toContain("● Rerail is on in acme-web. No rule packs are on yet.");
    expect(text).toContain("Rule packs     none yet  add one in the dashboard if it suits this repo");
    expect(text).not.toContain("0 rule packs");
  });
});

describe("C6b tips", () => {
  it.each(LANES)("explains Log, Steer and Block, says Block is not in this version, and lists three commands (%s)", lane => {
    const text = stripAnsi(renderTips(lane));
    expect(text).toContain("Three things can happen when a rule is broken.");
    expect(text).toContain("Log            Rerail records the violation for you. Your coding agent isn’t told.");
    expect(text).toContain("Steer          Your coding agent is told as it works, and fixes it. It isn’t stopped.");
    expect(text).toContain("Block          Your coding agent is stopped before it acts.  not in this version");
    expect(text).toContain("Each rule starts in Steer or Log, as its rule pack says. See each rule in the dashboard.");
    expect(text.match(/^ {2}\/rerail\S*/gm)).toEqual(["  /rerail", "  /rerail-create", "  /rerail-stop"]);
    expect(text).toContain("Rerail never edits your instruction files.");
  });

  it("opens with a line for the person in the agent lane", () => {
    expect(renderTips("agent").startsWith("A few things for your first session.\n")).toBe(true);
  });
});

describe("E1 coding agent can't install", () => {
  it("says nothing was changed and hands over the terminal command", () => {
    const text = renderCannotInstall({ installCommand: "npx rerail" }, "agent");
    expect(text).toContain("✕ Blocked: this session can’t write outside the project folder.");
    expect(text).toContain("Rerail installs into your home folder, so it stopped before changing anything.");
    expect(text).toContain("I couldn’t install Rerail from here. This session is sandboxed, and nothing was changed.");
    expect(text).toContain("Run this in your own terminal instead. It takes a few minutes:");
    expect(text.trimEnd().endsWith("\nnpx rerail")).toBe(true);
  });

  it("names a different blocker when given one", () => {
    expect(renderCannotInstall({ installCommand: "npx rerail", reason: "a company policy blocks npx" }, "agent")).toContain(
      "✕ Blocked: a company policy blocks npx.",
    );
  });
});

describe("E3 no repo here", () => {
  it.each(LANES)("asks which project to start with, folder first (%s)", lane => {
    const text = stripAnsi(renderNoRepoQuestion({}, lane));
    expect(text).toContain("△ No repo found here.  Rerail needs one project to read.");
    expect(text).toContain("Which project do you want to start with?");
    expect(text).toContain("Rerail reads it on this computer only, to recommend rule packs.");
    expect(text).toContain("1. Type the folder…  for example “~/code/acme-web”");
    expect(text).toContain("2. I’ll pick rule packs myself  Rerail won’t read any code");
    expect(text).toContain("3. Skip for now  start Rerail later, in a session");
  });

  it("tells the coding agent how to pass each answer", () => {
    const text = renderNoRepoQuestion({}, "agent");
    expect(NO_REPO_ANSWERS).toEqual(["folder", "pick-myself", "skip"]);
    expect(text).toContain("1. rerail setup --repo <the folder they name>");
    expect(text).toContain("2. rerail setup --answer pick-myself");
    expect(text).toContain("3. rerail setup --answer skip");
  });
});

describe("E4 pick rule packs myself", () => {
  it.each(LANES)("says no rule packs are on and points to the dashboard (%s)", lane => {
    const text = stripAnsi(renderPickedMyself({ repoName: "acme-web", connected: [CLAUDE, CURSOR], dashboard: OPENED }, lane));
    expect(text).toContain("● Rerail is on in acme-web. No rule packs are on yet.");
    expect(text).not.toMatch(/rule packs? (is|are) on, with/);
    expect(text).toContain("Connected      Claude Code and Cursor");
    expect(text).toContain("Your own       /rerail-create  create a rule in your own words, in a session");
    expect(text).toContain("Next           Turn on the rule packs you want.  then open a new session");
    expect(text).toContain("Manage rules   http://127.0.0.1:7777/#rules  opening in your browser");
    expect(text).toContain("Rerail did not read this repo. Changed your mind? Ask it to recommend rule packs, in the dashboard.");
  });

  it("echoes the choice in the terminal only", () => {
    const echo = "You chose to pick rule packs yourself. Rerail did not read any code.";
    expect(stripAnsi(renderPickedMyself({ repoName: "acme-web", connected: [CLAUDE], dashboard: OPENED }, "terminal"))).toContain(echo);
    expect(renderPickedMyself({ repoName: "acme-web", connected: [CLAUDE], dashboard: OPENED }, "agent")).not.toContain(echo);
  });
});

describe("E5 skip for now", () => {
  it.each(LANES)("says Rerail is on in no repo and how to start it later (%s)", lane => {
    const text = stripAnsi(renderSkipped({ connected: [CLAUDE, CURSOR] }, lane));
    expect(text).toContain("● Rerail is installed. It is not on in any repo yet.");
    expect(text).toContain("Connected      Claude Code and Cursor");
    expect(text).toContain("Next: open a session in one of your repos and type /rerail-start.");
    expect(text).toContain("Rerail then sets itself up for that repo: it asks, shows the rule packs that look right, and starts.");
  });
});

describe("E6 no browser", () => {
  it.each(LANES)("still shows the link, and one line saying no browser opened (%s)", lane => {
    const text = stripAnsi(renderSummary({ ...SUMMARY, dashboard: NOT_OPENED }, lane));
    expect(text).toContain("Manage rules   http://127.0.0.1:7777/#rules  open this link");
    expect(text).toContain("△ Couldn’t open a browser from here (this looks like a remote session).");
    expect(text).not.toContain("opening in your browser");
  });

  it("applies to the pick-myself summary too, and leaves out the reason when none is known", () => {
    const text = renderPickedMyself({ repoName: "acme-web", connected: [CLAUDE], dashboard: { url: "http://x/#rules", browserOpened: false } }, "agent");
    expect(text).toContain("Manage rules   http://x/#rules  open this link");
    expect(text).toContain("△ Couldn’t open a browser from here.");
  });
});
