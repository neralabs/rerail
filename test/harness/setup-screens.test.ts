import { describe, expect, it } from "vitest";
import {
  CONFIRM_PACKS_ANSWERS,
  NO_REPO_ANSWERS,
  READ_REPO_ANSWERS,
  readRepoQuestion,
  renderCannotInstall,
  renderBanner,
  renderInstalled,
  renderOnPath,
  renderAnswerHint,
  renderMenu,
  pickPacksQuestion,
  recommendedPacksQuestion,
  renderPacksPrompt,
  renderQuestion,
  renderQuestionHead,
  renderSessionsDeclined,
  sessionsQuestion,
  renderNoRepoQuestion,
  renderNodeTooOld,
  renderNotNow,
  renderReadRepoQuestion,
  renderRecommendedPacks,
  renderSkipped,
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
const OPENED: DashboardLink = { url: "http://127.0.0.1:7777/#rules", browserOpened: true };
const NOT_OPENED: DashboardLink = { url: "http://127.0.0.1:7777/#rules", browserOpened: false, noBrowserReason: "this looks like a remote session" };

const PACKS: RecommendedPacksInput = {
  repoName: "acme-web",
  repoPath: "/Users/me/code/acme-web",
  packs: [
    { id: "typescript", name: "TypeScript", ruleCount: 74, reason: "matches the typescript dependency in package.json and *.ts files", packageName: "@neralabs/rerail-pack-typescript", version: "0.1.1", installed: null },
    { id: "acme-api", name: "acme-api", ruleCount: 92, reason: "matches the acme_api package", packageName: "@neralabs/rerail-pack-acme-api", version: "1.2.0", installed: "1.2.0" },
  ],
  available: [
    { id: "typescript", version: "0.1.1", name: "TypeScript", reason: "matches the typescript dependency in package.json and *.ts files" },
    { id: "acme-api", version: "1.2.0", name: "acme-api", reason: "matches the acme_api package" },
    { id: "python", version: "0.1.2", name: "Python" },
  ],
  agentsFound: [CLAUDE, CURSOR],
};

const SUMMARY: SummaryInput = {
  repoName: "acme-web",
  packNames: ["TypeScript", "acme-api"],
  ruleCount: 166,
  dashboard: OPENED,
};

const LANES: readonly SetupLane[] = ["terminal", "agent"];

/** Every screen, rendered in one lane, so lane-wide properties can be checked once. */
function everyScreen(lane: SetupLane): Record<string, string> {
  return {
    T2: renderInstalled(lane),
    T2p: renderOnPath({ binDir: "/Users/me/.rerail/bin", locations: ["/Users/me/.zshrc"], home: "/Users/me" }, lane),
    T2b: renderQuestion(sessionsQuestion({ carriedArgs: [] }), lane),
    T2x: renderSessionsDeclined(lane),
    T3: renderReadRepoQuestion({ repoName: "acme-web", repoPath: "/Users/me/code/acme-web" }, lane),
    T4: renderRecommendedPacks(PACKS, lane),
    T5: renderNotNow(lane),
    T6: renderSummary(SUMMARY, lane),
    C6b: renderTips(lane),
    E1: renderCannotInstall({ installCommand: "npx rerail" }, lane),
    E2: renderNodeTooOld({ found: "20.11.1", requiredMajor: 24 }, lane),
    E3: renderNoRepoQuestion({}, lane),
    E5: renderSkipped(lane),
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
    const text = renderSummary({ ...SUMMARY, repoName: "acme\u001b]8;;evil\u0007-web" }, "agent");
    expect(text).toContain("Rerail is on in acme]8;;evil-web ·");
    expect(text).not.toMatch(/[\u001b\u0007]/);
  });
});

describe("T2 installed", () => {
  it("says Rerail is installed, nothing is on yet and the code is untouched, leaving the version to the banner", () => {
    expect(stripAnsi(renderInstalled("terminal"))).toBe(">^ᴥ^<  Rerail installed · nothing is on yet, your code is untouched\n");
    expect(renderInstalled("agent")).toBe("Rerail is installed. Nothing is on yet, and your code is untouched.\n");
  });
});

describe("T2 on PATH", () => {
  it("names the launcher and the startup file from ~, and only a whole home folder becomes ~", () => {
    const input = { binDir: "/Users/me/.rerail/bin", locations: ["/Users/me/.bashrc", "/Users/meg/.profile"], home: "/Users/me/" };
    expect(stripAnsi(renderOnPath(input, "terminal"))).toBe("● You can run `rerail` from any new terminal\n");
    expect(renderOnPath(input, "agent")).toBe("Added ~/.rerail/bin to PATH in ~/.bashrc and /Users/meg/.profile. Open a new terminal to run rerail.\n");
    expect(renderOnPath({ binDir: String.raw`C:\Users\me\.rerail\bin`, locations: [String.raw`HKCU\Environment Path`], home: String.raw`C:\Users\me` }, "agent"))
      .toContain(String.raw`Added ~\.rerail\bin to PATH in HKCU\Environment Path.`);
  });

  it("says what to do when it could not", () => {
    const text = stripAnsi(renderOnPath({ binDir: "/Users/me/.rerail/bin", locations: [], failure: "EACCES: permission denied", home: "/Users/me" }, "terminal"));
    expect(text).toBe("△ Couldn’t add ~/.rerail/bin to PATH (EACCES: permission denied). Add it yourself to run rerail from a terminal.\n");
  });
});

describe("T1 banner", () => {
  it("draws the footer's mark in a terminal, plain under NO_COLOR, and nothing for a coding agent", () => {
    expect(stripAnsi(renderBanner({ version: "0.3.0", depth: "truecolor" }, "terminal"))).toBe("\n  R E R A I L   set-up · 0.3.0\n\n");
    expect(renderBanner({ version: "0.3.0", depth: "none" }, "terminal")).not.toMatch(ANSI);
    expect(renderBanner({ version: "0.3.0", depth: "truecolor" }, "agent")).toBe("");
  });
});

describe("T2b may Rerail read sessions", () => {
  it("says what it sees and why, and its yes carries the folder the run was named", () => {
    const form = sessionsQuestion({ carriedArgs: ["--repo", "/Users/me/code/acme-web"] });
    const text = renderQuestion(form, "agent");
    expect(text).toContain("Can Rerail see your coding agent sessions?");
    expect(text).toContain("Rerail sees what you ask your agent, what it does, and the files it edits,\nso it can check the work against your rules.");
    expect(text).toContain("  ✓ Stays on this computer. Nothing is sent anywhere.");
    expect(text).toContain("  ✓ You’re asked once, for every repo you start Rerail in.");
    expect(text).toContain("1. Yes, allow\n");
    expect(text).toMatch(/2\. No, cancel set-up +nothing changes/);
    expect(text).toContain("1. rerail setup --answer allow --repo /Users/me/code/acme-web");
    expect(text).toContain("2. rerail setup --answer decline");
  });
});

describe("question steps", () => {
  const form = readRepoQuestion({ repoName: "acme-web", repoPath: "/Users/me/code/acme-web" });

  it("numbers the step before the question in the terminal, only when given one", () => {
    expect(stripAnsi(renderQuestion(form, "terminal", { index: 2, total: 3 }))).toContain("Step 2 of 3 · Find rule packs for acme-web?");
    expect(stripAnsi(renderQuestion(form, "terminal"))).not.toContain("Step");
  });

  it("leaves the step out for a coding agent", () => {
    expect(renderQuestion(form, "agent", { index: 2, total: 3 })).not.toContain("Step");
  });

  it("lines the hints up in one column past the longest answer", () => {
    const rows = stripAnsi(renderQuestion(form, "terminal")).split("\n");
    const other = rows.find(row => row.includes("2. Scan a different repo…")) ?? "";
    const skip = rows.find(row => row.includes("3. Skip, I’ll pick packs")) ?? "";
    expect(other.indexOf("e.g. ~/code/landing-page")).toBe(skip.indexOf("Rerail won’t scan"));
    expect(other).toMatch(/…    e\.g\./);
  });

  it("draws the terminal menu as the question draws its answers, with the cursor where it is moved", () => {
    const question = stripAnsi(renderQuestion(form, "terminal", { index: 2, total: 3 }));
    expect(`${stripAnsi(renderQuestionHead(form, { index: 2, total: 3 }))}${stripAnsi(renderMenu(form, 0))}`)
      .toBe(`${question}\n↑↓ to move, Enter to choose, or type a number: `);
    const rows = stripAnsi(renderMenu(form, 2)).split("\n");
    expect(rows).toHaveLength(form.choices.length + 2);
    expect(rows[0]).toMatch(/^  1\. Yes, scan acme-web/u);
    expect(rows[2]).toMatch(/^❯ 3\. Skip, I’ll pick packs/u);
    expect(rows.at(-2)).toBe("");
  });

  it("says which numbers a typed answer may be", () => {
    expect(stripAnsi(renderAnswerHint(3))).toBe("Type a number from 1 to 3.");
  });
});

describe("T3 / C3 ask before reading", () => {
  const input = { repoName: "acme-web", repoPath: "/Users/me/code/acme-web" };

  it.each(LANES)("names the repo, says what scanning does, and offers three answers (%s)", lane => {
    const text = stripAnsi(renderReadRepoQuestion(input, lane));
    expect(text).toContain("Find rule packs for acme-web?");
    expect(text).toContain("Rule packs are ready-made sets of rules your coding agent must follow.");
    expect(text).toContain("Rerail can scan this repo’s files and suggest packs that fit.");
    expect(text).toContain("  ✓ Scanning only reads files. Your code stays on this computer.");
    expect(text).toContain("  ✓ Nothing is installed or turned on until you say yes.");
    expect(text).toContain("1. Yes, scan acme-web");
    expect(text).toMatch(/2\. Scan a different repo… +e\.g\. ~\/code\/landing-page/);
    expect(text).toMatch(/3\. Skip, I’ll pick packs +Rerail won’t scan/);
  });

  it("puts the terminal cursor on the first answer", () => {
    expect(stripAnsi(renderReadRepoQuestion(input, "terminal"))).toContain("❯ 1. Yes, scan acme-web");
  });

  it("tells the coding agent to ask, not answer, and how to call set-up again for each answer", () => {
    const text = renderReadRepoQuestion(input, "agent");
    expect(READ_REPO_ANSWERS).toEqual(["yes", "other-folder", "pick-myself"]);
    expect(text).toContain("Ask them this, as written, and wait for their reply");
    expect(text).toContain("Do not choose for them.");
    expect(text).toContain("1. rerail setup --answer yes --repo /Users/me/code/acme-web");
    expect(text).toContain("2. rerail setup --repo <the folder they name>");
    expect(text).toContain("3. rerail setup --answer pick-myself --repo /Users/me/code/acme-web");
    expect(text.indexOf("1. Yes, scan acme-web")).toBeLessThan(text.indexOf("1. rerail setup"));
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
  it("keeps the blank line before what follows the question in --json, where the question goes out as data", () => {
    const text = renderQuestion(recommendedPacksQuestion(PACKS, "json"), "json");
    expect(text).toContain("Connecting is free.\n\nWhen your coding agent breaks a rule, Rerail tells it");
    expect(renderQuestion(sessionsQuestion({ carriedArgs: [] }), "json")).not.toMatch(/^\n/u);
  });

  it.each(LANES)("lists each rule pack first, then its rule count, then why it fits, then what a yes connects (%s)", lane => {
    const text = stripAnsi(renderRecommendedPacks(PACKS, lane));
    expect(text).toContain("Scanned acme-web. 2 rule packs fit:");
    expect(text).toMatch(/Rule pack\s+Rules\s+Why it fits/);
    expect(text).toMatch(/TypeScript\s+74\s+matches the typescript dependency in package\.json and \*\.ts files/);
    expect(text).toMatch(/acme-api\s+92\s+matches the acme_api package/);
    expect(text).not.toContain("Installs from npm");
    expect(text).not.toContain("replaces");
    expect(text).toContain("  Will connect to: Claude Code, Cursor\n  Adds /rerail commands and a status line to each. Connecting is free.");
    expect(text).toContain("Start Rerail in acme-web with these 2 rule packs?");
    expect(text).toContain("When your coding agent breaks a rule, Rerail tells it, and the agent fixes\nits own work. Rerail never edits your code itself.");
    expect(text).toContain("1. Yes, start Rerail");
    expect(text).toMatch(/2\. Change rule packs… +here or in the dashboard/);
    expect(text).toMatch(/3\. Not now +nothing is turned on/);
    expect(text).not.toMatch(/Review them first|Type something else|pick rule packs myself|4\. /);
  });

  it("offers start, change and not now, in that order", () => {
    expect(CONFIRM_PACKS_ANSWERS).toEqual(["start", "choose", "not-now"]);
  });

  it("carries the versions shown in the yes, lists every pack for choosing, and changes nothing on not now", () => {
    const text = renderRecommendedPacks(PACKS, "agent");
    expect(text).toContain("1. rerail setup --answer start --repo /Users/me/code/acme-web --packs typescript@0.1.1,acme-api@1.2.0");
    expect(text).toContain("2. To choose the rule packs: rerail setup --answer choose --repo /Users/me/code/acme-web --packs <packs>, with comma-separated packs from: typescript@0.1.1,acme-api@1.2.0,python@0.1.2.");
    expect(text).toContain("3. rerail setup --answer not-now\n");
    expect(text).not.toContain("--agents");
    expect(renderRecommendedPacks({ ...PACKS, packs: [] }, "agent")).toContain("1. rerail setup --answer start --repo /Users/me/code/acme-web\n");
  });

  it("leaves out Change rule packs… when there is no pack to choose from", () => {
    const text = stripAnsi(renderRecommendedPacks({ ...PACKS, packs: [], available: [] }, "terminal"));
    expect(text).not.toContain("Change rule packs");
    expect(text).toMatch(/2\. Not now/);
  });

  it("warns when a yes replaces a version every repo on this computer runs", () => {
    const text = stripAnsi(renderRecommendedPacks({ ...PACKS, packs: [{ ...PACKS.packs[1]!, installed: "1.1.0" }] }, "terminal"));
    expect(text).toContain("  △ @neralabs/rerail-pack-acme-api@1.2.0 replaces 1.1.0, which every repo on this computer uses.");
  });

  it("aligns the table columns for long rule pack names", () => {
    const text = renderRecommendedPacks({ ...PACKS, packs: [{ ...PACKS.packs[0]!, name: "A very long rule pack name", ruleCount: 12, reason: "the reason" }] }, "agent");
    const header = text.split("\n").find(line => line.includes("Rule pack")) ?? "";
    const row = text.split("\n").find(line => line.includes("A very long")) ?? "";
    expect(row.indexOf("12")).toBe(header.indexOf("Rules"));
    expect(row.indexOf("the reason")).toBe(header.indexOf("Why it fits"));
  });

  it("lists what the lint config did not read, then the rules it starts off or configures, counting past eight lines", () => {
    const settings = Array.from({ length: 8 }, (_, index) => ({ ruleId: `eslint-rule-${index}`, folder: index === 0 ? "tests" : "", on: false as const, config: null, because: `.eslintrc.json: "rule-${index}": "off"` }));
    const text = renderRecommendedPacks({
      ...PACKS,
      lintConfig: {
        settings: [{ ruleId: "typescript-eslint-consistent-type-definitions", folder: "", on: null, config: { option: "type" }, because: ".eslintrc.json: \"x\": [\"error\",\"type\"]" }, ...settings],
        notes: ["eslint.config.ts is not read: Rerail never runs your config."],
      },
    }, "agent");
    expect(text).toContain("  From your lint config (read as data, never run):\n    eslint.config.ts is not read: Rerail never runs your config.\n    typescript-eslint-consistent-type-definitions starts with option = \"type\"  .eslintrc.json: \"x\": [\"error\",\"type\"]\n    eslint-rule-0 starts off in tests/  .eslintrc.json: \"rule-0\": \"off\"\n    eslint-rule-1 starts off  .eslintrc.json: \"rule-1\": \"off\"");
    expect(text).toContain("    eslint-rule-4 starts off");
    expect(text).not.toContain("eslint-rule-5");
    expect(text).toContain("    …and 3 more.");
    expect(renderRecommendedPacks({ ...PACKS, lintConfig: { settings: [], notes: [] } }, "agent")).not.toContain("lint config");
  });

  it("marks the scan with a bullet in the terminal only", () => {
    expect(stripAnsi(renderRecommendedPacks(PACKS, "terminal"))).toContain("● Scanned acme-web. 2 rule packs fit:");
    expect(renderRecommendedPacks(PACKS, "agent")).not.toContain("● Scanned");
  });

  it("speaks of one rule pack in the singular", () => {
    const text = renderRecommendedPacks({ ...PACKS, packs: PACKS.packs.slice(0, 1) }, "agent");
    expect(text).toContain("Scanned acme-web. 1 rule pack fits:");
    expect(text).toContain("Start Rerail in acme-web with this rule pack?");
  });

  it("says plainly when no rule pack fits, and when no coding agent was found", () => {
    const text = renderRecommendedPacks({ ...PACKS, packs: [], agentsFound: [] }, "agent");
    expect(text).toContain("Scanned acme-web. None of Rerail’s rule packs fit it.");
    expect(text).not.toContain("Rule pack ");
    expect(text).toContain("Start Rerail in acme-web with no rule packs on?");
    expect(text).toContain("  Will connect to: no coding agents found");
    expect(text).not.toContain("Adds /rerail commands");
  });
});

describe("T5 not now", () => {
  it.each(LANES)("says nothing was installed or turned on, and how to start later (%s)", lane => {
    const text = stripAnsi(renderNotNow(lane));
    expect(text).toContain("● Set-up stopped. No rule packs were installed and nothing was turned on.");
    expect(text).toContain(`Start Rerail in this repo whenever you’re ready with ${lane === "terminal" ? "rerail start" : "/rerail-start"}.`);
  });
});

describe("T6 / C6 summary", () => {
  it.each(LANES)("has every decided line, ending on the link and the other repos (%s)", lane => {
    const text = stripAnsi(renderSummary(SUMMARY, lane));
    expect(text).toContain("● Rerail is on in acme-web · 2 rule packs, 166 rules\n");
    expect(text).toContain("  Try it    Open a new agent session and type /rerail-show-me\n            to watch Rerail catch and fix a broken rule.\n");
    expect(text).toContain("  Manage    http://127.0.0.1:7777/#rules\n            Opening in your browser.\n");
    expect(text).toContain("  Rerail is only on in acme-web. To add another repo, type /rerail-start there.");
    expect(text).toContain("  Type /rerail in any session for help and every command.");
    expect(text).not.toMatch(/Connected|Rule packs {2}|Your own/);
    expect(text).not.toMatch(/close this/i);
  });

  it("puts the tips below the divider, after the link, in the terminal", () => {
    const text = stripAnsi(renderSummary(SUMMARY, "terminal"));
    const link = text.indexOf("Manage");
    const divider = text.indexOf("── Good to know ──");
    expect(divider).toBeGreaterThan(link);
    expect(text).not.toContain("optional");
    expect(text.indexOf("When a rule is broken, Rerail can:")).toBeGreaterThan(divider);
  });

  it("dims every tip line in the terminal", () => {
    const tips = renderTips("terminal").trimEnd().split("\n");
    for (const line of tips) expect(line).toMatch(/^\u001b\[2m.*\u001b\[22m$/);
  });

  it("offers the tips in the agent lane and keeps them for when the person asks", () => {
    const text = renderSummary(SUMMARY, "agent");
    const offer = text.indexOf("Want a few tips for your first session? Just ask.");
    expect(offer).toBeGreaterThan(text.indexOf("Manage"));
    expect(text.indexOf("Only if they ask for tips")).toBeGreaterThan(offer);
    expect(text.indexOf("A few things for your first session.")).toBeGreaterThan(offer);
    expect(text).not.toContain("── Good to know");
  });

  it("names other repos where Rerail is already on, instead of saying it is only on here", () => {
    const text = renderSummary({ ...SUMMARY, otherStartedRepos: ["landing-page", "docs"] }, "agent");
    expect(text).toContain("  Rerail is also on in landing-page and docs. To add another repo, type /rerail-start there.");
    expect(text).not.toContain("only on in");
  });

  it("speaks of one rule pack and one rule in the singular", () => {
    expect(renderSummary({ ...SUMMARY, packNames: ["TSLint"], ruleCount: 1 }, "agent")).toContain("Rerail is on in acme-web · 1 rule pack, 1 rule\n");
  });

  it("says no rule packs are on, not zero of them, where none fit the repo", () => {
    const text = renderSummary({ ...SUMMARY, packNames: [], ruleCount: 0 }, "agent");
    expect(text).toContain("● Rerail is on in acme-web · no rule packs on yet\n");
    expect(text).not.toContain("0 rule packs");
  });

  it("shows the link alone when an earlier run opened it", () => {
    const text = renderSummary({ ...SUMMARY, dashboard: { url: "http://x/#rules", browserOpened: false, linkOnly: true } }, "agent");
    expect(text).toContain("  Manage    http://x/#rules\n\n");
    expect(text).not.toMatch(/browser/);
  });
});

describe("C6b tips", () => {
  it.each(LANES)("explains Steer and Log only, and lists /rerail-create and /rerail-stop (%s)", lane => {
    const text = stripAnsi(renderTips(lane));
    expect(text).toContain("  When a rule is broken, Rerail can:\n    Steer   tell your agent, so it fixes it      most rules start here\n    Log     only record it, for you to review\n  Change a rule’s mode in the dashboard.");
    expect(text).not.toContain("Block");
    expect(text.match(/^ {2}\/rerail\S*/gm)).toEqual(["  /rerail-create", "  /rerail-stop"]);
    expect(text).toContain("  /rerail-create   write your own rule, in plain words");
    expect(text).toContain("  /rerail-stop     stop Rerail in this repo");
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
    expect(text).toMatch(/1\. Type the folder… +for example “~\/code\/acme-web”/);
    expect(text).toMatch(/2\. Skip for now +start Rerail later, in a session/);
    expect(text).not.toContain("pick rule packs");
  });

  it("tells the coding agent how to pass each answer", () => {
    const text = renderNoRepoQuestion({}, "agent");
    expect(NO_REPO_ANSWERS).toEqual(["folder", "skip"]);
    expect(text).toContain("1. rerail setup --repo <the folder they name>");
    expect(text).toContain("2. rerail setup --answer skip");
    expect(text).not.toContain("pick-myself");
  });
});

describe("E4 skip, I’ll pick packs", () => {
  const input = { repoName: "acme-web", repoPath: "/Users/me/code/acme-web", available: PACKS.available, unavailable: [] };

  it("lists the packs once in the terminal, numbered at the prompt after “Choose rule packs…”", () => {
    const question = stripAnsi(renderQuestion(pickPacksQuestion(input, "terminal"), "terminal"));
    expect(question).not.toContain("python@0.1.2");
    expect(question).toContain("Which rule packs should Rerail start with in acme-web?");
    expect(stripAnsi(renderPacksPrompt({ available: PACKS.available }))).toContain("  3. python@0.1.2  Python");
  });

  it.each(["agent", "json"] as const)("lists them in the question where no prompt follows (%s)", lane => {
    const text = renderQuestion(pickPacksQuestion(input, lane), lane);
    expect(text).toContain("  Rerail’s rule packs, from npm:\n    typescript@0.1.1  TypeScript");
    expect(text).toContain("    python@0.1.2  Python");
  });
});

describe("E5 skip for now", () => {
  it.each(LANES)("says Rerail is on in no repo and how to start it later, as the summary is laid out (%s)", lane => {
    const text = stripAnsi(renderSkipped(lane));
    expect(text).toContain("● Rerail is installed · not on in any repo yet\n\n");
    expect(text).toContain("  Start     Open a session in one of your repos and type /rerail-start.\n            Rerail asks, suggests rule packs that fit, and starts there.");
    expect(text).toContain("  Type /rerail in any session for help and every command.");
    expect(text).not.toMatch(/Connected|Next:/);
  });

  it("is handed to the person as the summary is, by a coding agent", () => {
    expect(renderSkipped("agent")).toMatch(/^Rerail set-up is done\. Show the person this summary, as written:\n\n● Rerail is installed/u);
  });
});

describe("E6 no browser", () => {
  it.each(LANES)("still shows the link, and one line saying no browser opened (%s)", lane => {
    const text = stripAnsi(renderSummary({ ...SUMMARY, dashboard: NOT_OPENED }, lane));
    expect(text).toContain("  Manage    http://127.0.0.1:7777/#rules\n            △ Couldn’t open your browser. Copy the link instead.");
    expect(text).not.toContain("Opening in your browser");
  });
});
