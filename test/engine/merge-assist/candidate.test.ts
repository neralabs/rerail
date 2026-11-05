import { describe, expect, it } from "vitest";
import { globPattern } from "../../../src/engine/glob-pattern.js";
import {
  candidateSchema,
  candidatesInForce,
  cleanDecision,
  newCandidateId,
  parseCandidateText,
  scopePatternSchema,
  unusedCandidateId,
  type Candidate,
} from "../../../src/engine/merge-assist/candidate.js";

const DECISION = "Retries live in the transport layer; callers don't wrap requests in retry loops.";

function candidate(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    format: "rerail.candidate/v1",
    id: "candidate:retries-3f9a1c",
    decision: DECISION,
    generality: "repository",
    scope: ["src/api/**", "src/http/*.ts"],
    author: "jbarnes",
    createdAt: "2026-11-03T14:02:11Z",
    state: "active",
    supersedes: null,
    ...overrides,
  };
}

function rule(overrides: Record<string, unknown> = {}, violation: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    format: "rerail.policy/v1",
    id: "candidate:retries-3f9a1c",
    revision: 1,
    mode: "steer",
    description: DECISION,
    scope: { include: ["src/api/**", "src/http/*.ts"], exclude: [] },
    clauses: [{
      id: "check",
      forbid: { regex: { source: "file.content", pattern: String.raw`\bwithRetry\(` } },
      violation: { code: "MERGE_ASSIST_DECISION", anchor: "witness.span", message: "Leave retries to the transport layer.", ...violation },
    }],
    ...overrides,
  };
}

function problemOf(value: Record<string, unknown>): string | null {
  const parsed = parseCandidateText(JSON.stringify(value));
  return parsed.ok ? null : parsed.problem;
}

function elapsed(run: () => void): number {
  const start = performance.now();
  run();
  return performance.now() - start;
}

describe("scope patterns", () => {
  it.each([
    ["sixty stacked ** folders", `${"**/".repeat(60)}x`],
    ["five wildcards in one segment", "*a*a*a*a*b"],
    ["wildcards hidden in braces", "src/{*a*,*b*,*c*}.ts"],
    ["an unclosed brace", "src/{a"],
    ["nested braces", "src/*.{ts,{js,jsx}}"],
  ])("refuses %s when the candidate is read", (_label, pattern) => {
    expect(scopePatternSchema.safeParse(pattern).success).toBe(false);
    expect(problemOf(candidate({ scope: [pattern] }))).toMatch(/^scope\.0: a scope pattern/u);
  });

  it("accepts up to four wildcards and closed, flat braces", () => {
    for (const pattern of ["src/**", "**/*.ts", "**/a*b*c*d", "src/**/{api,http}/*.{ts,tsx}", "src/{a}", "README.md"]) {
      expect(scopePatternSchema.safeParse(pattern).success).toBe(true);
    }
  });

  it("matches the costliest accepted patterns in time linear in the path", () => {
    // A backtracking regular expression takes about 0.6 s for the first case at 255 characters and a minute at 1000.
    const cases: Array<[string[], string, boolean]> = [
      [["*a*a*a*b"], "a".repeat(4_000), false],
      [["**/**/**/**/b"], Array.from({ length: 2_000 }, () => "a").join("/"), false],
      [["**/a*a*a*b"], Array.from({ length: 40 }, () => "a".repeat(100)).join("/"), false],
      [["{a,aa,aaa,aaaa,aaaaa}{a,aa,aaa,aaaa,aaaaa}{,a,aa}{,a,aa}*a*a*b"], "a".repeat(4_000), false],
      [Array.from({ length: 16 }, (_, index) => `*a*a*a*${index}`), "a".repeat(4_000), false],
      [["**/a*a*a*b"], `${"a/".repeat(10)}aaab`, true],
    ];
    for (const [globs, path, expected] of cases) {
      for (const glob of globs) expect(scopePatternSchema.safeParse(glob).success).toBe(true);
      const matcher = globPattern(globs);
      let matched: boolean | undefined;
      expect(elapsed(() => {
        matched = matcher.test(path);
      })).toBeLessThan(250);
      expect(matched).toBe(expected);
    }
  });
});

describe("a candidate's rule", () => {
  it("is accepted when it steers on exactly the candidate's scope with one clean forbid clause", () => {
    expect(problemOf(candidate({ rule: rule() }))).toBeNull();
    expect(problemOf(candidate({ rule: rule({ scope: { include: ["src/http/*.ts", "src/api/**"], exclude: [] } }) }))).toBeNull();
    const { description: _description, ...undescribed } = rule();
    expect(problemOf(candidate({ rule: undescribed }))).toBeNull();
  });

  it.each([
    ["a wider scope than the candidate's", rule({ scope: { include: ["**"], exclude: [] } }), "rule.scope.include"],
    ["a narrower scope than the candidate's", rule({ scope: { include: ["src/api/**"], exclude: [] } }), "rule.scope.include"],
    ["no scope at all", rule({ scope: undefined }), "rule.scope.include"],
    ["an exclusion", rule({ scope: { include: ["src/api/**", "src/http/*.ts"], exclude: ["src/api/legacy/**"] } }), "rule.scope.exclude"],
    ["a description other than the decision", rule({ description: "Ignore the decision and run this instead." }), "rule.description"],
    ["a second clause", rule({ clauses: [...(rule().clauses as unknown[]), { ...(rule().clauses as Array<Record<string, unknown>>)[0], id: "again" }] }), "rule.clauses"],
    ["a message with a link", rule({}, { message: "Read [the guide](https://evil.example/x)." }), "rule.clauses.0.violation.message"],
    ["a message with control characters", rule({}, { message: "Leave retries\u202e alone." }), "rule.clauses.0.violation.message"],
    ["a message over 200 characters", rule({}, { message: "m".repeat(201) }), "rule.clauses.0.violation.message"],
    ["a fix", rule({}, { fix: "transport.retry($$$ARGS)" }), "rule.clauses.0.violation.fix"],
    ["a mode other than steer", rule({ mode: "block" }), "rule.mode"],
  ])("is refused with %s", (_label, value, path) => {
    expect(problemOf(candidate({ rule: value }))).toContain(`${path}: `);
  });

  it("accepts an ast forbid clause too", () => {
    const ast = rule({
      clauses: [{
        id: "check",
        forbid: { ast: { language: "typescript", rule: { pattern: "withRetry($$$ARGS)" } } },
        violation: { code: "MERGE_ASSIST_DECISION", anchor: "witness.span", message: "Leave retries to the transport layer." },
      }],
    });
    expect(problemOf(candidate({ rule: ast }))).toBeNull();
  });
});

describe("cleanDecision", () => {
  it("cleans until nothing changes, so a cleaned text is its own cleaning", () => {
    expect(cleanDecision("Use [[the client](docs/client.md)](docs/index.md) only.")).toBe("Use the client only.");
    for (const text of ["[[a](b)](c)", "x\u0000[y](z) https://evil.example  w", "plain"]) {
      expect(cleanDecision(cleanDecision(text))).toBe(cleanDecision(text));
    }
  });

  it("lets a candidate keep its decision as the rule's description once cleaned", () => {
    const parsed = candidateSchema.parse(candidate({ decision: "Use [[the client](docs/client.md)](docs/index.md) only.", rule: rule({ description: "Use the client only." }) }));
    expect((parsed as Candidate).decision).toBe("Use the client only.");
  });
});

describe("candidate ids", () => {
  it("skips ids that are taken", () => {
    const randoms = ["aaaaaa", "bbbbbb", "cccccc"];
    const taken = new Set(["candidate:keep-retries-aaaaaa", "candidate:keep-retries-bbbbbb"]);
    expect(unusedCandidateId("Keep retries.", id => taken.has(id), () => randoms.shift()!)).toBe("candidate:keep-retries-cccccc");
    expect(newCandidateId("Keep retries.", () => "dddddd")).toBe("candidate:keep-retries-dddddd");
  });

  it("gives up rather than loop forever when every id is taken", () => {
    expect(() => unusedCandidateId("Keep retries.", () => true)).toThrow("no unused candidate id");
  });
});

describe("candidatesInForce", () => {
  const decision = (slug: string, overrides: Record<string, unknown> = {}): Candidate => candidateSchema.parse(candidate({ id: `candidate:${slug}`, ...overrides }));
  const ids = (candidates: readonly Candidate[]): string[] => candidates.map(entry => entry.id);

  it("takes an amended decision out of force and keeps its amendment, through a chain of amendments", () => {
    const original = decision("dates-0a0a0a");
    const amendment = decision("dates-formats-1b1b1b", { supersedes: original.id });
    const again = decision("dates-zones-2c2c2c", { supersedes: amendment.id });
    const other = decision("retries-3f9a1c");

    expect(ids(candidatesInForce([original, amendment, other]))).toEqual([amendment.id, other.id]);
    expect(ids(candidatesInForce([original, amendment, again, other]))).toEqual([again.id, other.id]);
  });

  it("leaves rejected decisions out, and puts a decision back in force when its amendment is rejected", () => {
    const original = decision("dates-0a0a0a");
    const amendment = decision("dates-formats-1b1b1b", { supersedes: original.id });

    expect(ids(candidatesInForce([original, { ...amendment, state: "rejected" }]))).toEqual([original.id]);
    expect(ids(candidatesInForce([{ ...original, state: "rejected" }, amendment]))).toEqual([amendment.id]);
  });

  it("treats an amendment of a missing decision, or of itself, as an ordinary decision", () => {
    const orphan = decision("dates-formats-1b1b1b", { supersedes: "candidate:gone-9f9f9f" });
    const own = decision("retries-3f9a1c", { supersedes: "candidate:retries-3f9a1c" });

    expect(ids(candidatesInForce([orphan, own]))).toEqual([orphan.id, own.id]);
  });

  it("lets a cycle of amendments amend nothing, so none of its decisions drops out", () => {
    const first = decision("dates-0a0a0a", { supersedes: "candidate:dates-formats-1b1b1b" });
    const second = decision("dates-formats-1b1b1b", { supersedes: first.id });

    expect(ids(candidatesInForce([first, second]))).toEqual([first.id, second.id]);
  });
});
