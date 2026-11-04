// scripts/feedback/digest.mjs: the weekly feedback digest's reading of issue forms and its weekly counts.

import { describe, expect, it } from "vitest";
import { misfireKind, parseForm, ruleId, summarize } from "../../scripts/feedback/digest.mjs";

const week = { start: new Date("2026-10-19T00:00:00Z"), end: new Date("2026-10-26T00:00:00Z") };

/** A rule misfire issue as GitHub's REST API returns one written through the form. */
const misfireIssue = (number, { kind, rule, pack, created = "2026-10-20T10:00:00Z", association = "NONE", state = "open" }) => ({
  number,
  title: `[rule] ${rule}`,
  html_url: `https://github.com/neralabs/rerail/issues/${number}`,
  state,
  created_at: created,
  closed_at: null,
  author_association: association,
  labels: [{ name: "rule-misfire" }, { name: "needs-triage" }],
  body: `### What went wrong\n\n${kind}\n\n### Rule id\n\n${rule}\n\n### Pack and version\n\n${pack}\n\n### The steer the agent got\n\n_No response_\n`,
});

describe("parseForm", () => {
  it("reads each field's answer and leaves out the ones GitHub marks _No response_", () => {
    const form = parseForm("### Rule id\r\n\r\neslint-eqeqeq\r\n\r\n### The steer the agent got\r\n\r\n_No response_\r\n");
    expect([...form]).toEqual([["Rule id", "eslint-eqeqeq"]]);
  });

  it("keeps an answer's own Markdown headings of other levels inside it", () => {
    expect(parseForm("### Code\n\n```ts\n#### not a field\nlet a = 1;\n```\n").get("Code")).toBe("```ts\n#### not a field\nlet a = 1;\n```");
  });
});

describe("misfireKind and ruleId", () => {
  it("maps the form's answers to the three kinds, and anything else to not given", () => {
    expect(misfireKind("False positive (it flagged code that is fine)")).toBe("False positive");
    expect(misfireKind("False negative (it missed code it should flag)")).toBe("Missed violation");
    expect(misfireKind("The steer was wrong or unhelpful")).toBe("Confusing steer");
    expect(misfireKind(undefined)).toBe("not given");
  });

  it("takes the rule id from the title only when the form gives none and the title starts with one", () => {
    expect(ruleId(new Map([["Rule id", "eslint-eqeqeq"]]), "typescript-eslint:member-ordering: x")).toBe("eslint-eqeqeq");
    expect(ruleId(new Map(), "typescript-eslint:member-ordering: files fail to parse")).toBe("typescript-eslint:member-ordering");
    expect(ruleId(new Map(), "n-hashbang: shebangs are flagged")).toBe("n-hashbang");
    expect(ruleId(new Map(), "bandit:b201 / b202: not reported")).toBe("not given");
    expect(ruleId(new Map(), "react (all rules): run on Vue code")).toBe("not given");
  });
});

describe("summarize", () => {
  it("counts only issues opened inside the week, and leaves pull requests out", () => {
    const digest = summarize(
      [
        misfireIssue(1, { kind: "False positive (it flagged code that is fine)", rule: "eslint-eqeqeq", pack: "javascript@0.1.4", created: "2026-10-18T23:59:59Z" }),
        misfireIssue(2, { kind: "False positive (it flagged code that is fine)", rule: "eslint-eqeqeq", pack: "javascript@0.1.4", created: "2026-10-19T00:00:00Z" }),
        misfireIssue(3, { kind: "False positive (it flagged code that is fine)", rule: "eslint-eqeqeq", pack: "javascript@0.1.5", created: "2026-10-26T00:00:00Z" }),
        { ...misfireIssue(4, { kind: "x", rule: "eslint-eqeqeq", pack: "javascript@0.1.4" }), pull_request: { url: "…" } },
      ],
      week,
    );
    expect(digest.opened.map((issue) => issue.number)).toEqual([2]);
    expect(digest.open).toBe(3);
    expect(digest.needsTriage).toBe(3);
  });

  it("groups a week's misfires by rule and pack, without the pack's version, and tells outside reporters from the team", () => {
    const digest = summarize(
      [
        misfireIssue(1, { kind: "False positive (it flagged code that is fine)", rule: "eslint-eqeqeq", pack: "javascript@0.1.4" }),
        misfireIssue(2, { kind: "The steer was wrong or unhelpful", rule: "eslint-eqeqeq", pack: "javascript@0.1.5", association: "MEMBER" }),
        misfireIssue(3, { kind: "False negative (it missed code it should flag)", rule: "my-rule", pack: "own rule", state: "closed" }),
      ],
      week,
    );
    expect(digest.outside.map((issue) => issue.number)).toEqual([1, 3]);
    expect(Object.fromEntries(digest.misfires.kinds)).toEqual({ "False positive": 1, "Confusing steer": 1, "Missed violation": 1 });
    expect(Object.fromEntries(digest.misfires.packs)).toEqual({ javascript: 2, "own rule": 1 });
    expect(digest.misfires.rules.map((rule) => [rule.rule, rule.pack, rule.issues.map((issue) => issue.number)])).toEqual([
      ["eslint-eqeqeq", "javascript", [1, 2]],
      ["my-rule", "own rule", [3]],
    ]);
    expect(digest.openMisfires.total).toBe(2);
  });
});
