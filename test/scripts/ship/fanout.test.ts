// scripts/ship/fanout.ts: how the fan-out cuts the three slices into units and spreads them over hosts, and how it
// merges the hosts' runs. Slice A is cut by repetition (its cell's repetitions on one host, so its confirmation there
// sees them all), slice B into chunks of rules within the unit budget, slice C by file; groups go longest first onto
// the host that would finish them soonest. The merge concatenates a cell's logs from every host and copies the rest.

import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { claimName, detached, mergeTrees, type PackInput, planFanout, planUnits, remoteShell, UNIT_BUDGET_S } from "../../../scripts/ship/fanout.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

const rules = (n: number): string[] => ["(none)", ...Array.from({ length: n - 1 }, (_, i) => `r${String(i).padStart(3, "0")}`)];
const python: PackInput = { pack: "python", corpora: ["fixtures", "real"], rules: rules(539), files: ["largest", "median", "small"] };
const sql: PackInput = { pack: "sql", corpora: ["real"], rules: rules(22), files: ["largest", "median", "small"] };
const options = { runs: 3, ruleRuns: 2, fileRuns: 3 };

describe("planning units", () => {
  it("cuts slice A by repetition, so an A cell's repetitions run on several workers", () => {
    const a = planUnits([python], options).filter((u) => u.slice === "A" && u.corpus === "real");
    expect(a.map((u) => [u.id, u.rep, u.group])).toEqual([
      ["A|python|real|r1", 1, "A|python|real"],
      ["A|python|real|r2", 2, "A|python|real"],
      ["A|python|real|r3", 3, "A|python|real"],
    ]);
  });

  it("cuts slice B into chunks of rules within the unit budget, covering every rule once, the empty set first", () => {
    const b = planUnits([python], options).filter((u) => u.slice === "B" && u.corpus === "real");
    expect(b.length).toBeGreaterThan(1);
    expect(Math.max(...b.map((u) => u.est))).toBeLessThanOrEqual(UNIT_BUDGET_S);
    expect(b.flatMap((u) => u.rules!)).toEqual(python.rules);
    expect(b[0]!.rules![0]).toBe("(none)");
  });

  it("makes one unit per slice C file, on real code", () => {
    expect(planUnits([sql], options).filter((u) => u.slice === "C").map((u) => [u.id, u.corpus, u.label])).toEqual([
      ["C|sql|largest", "real", "largest"],
      ["C|sql|median", "real", "median"],
      ["C|sql|small", "real", "small"],
    ]);
  });

  it("plans no fixtures units for a pack without a fixtures corpus", () => {
    expect(planUnits([sql], options).some((u) => u.corpus === "fixtures")).toBe(false);
  });
});

describe("spreading units over hosts", () => {
  const plan = planFanout([python, sql], ["build-1", "build-2"], ["0-3", "4-7", "8-11", "12-15"], options);

  it("assigns every unit once, longest first", () => {
    expect(plan.units).toHaveLength(planUnits([python, sql], options).length);
    expect(new Set(plan.units.map((u) => u.id)).size).toBe(plan.units.length);
    for (let i = 1; i < plan.units.length; i += 1) expect(plan.units[i - 1]!.est).toBeGreaterThanOrEqual(plan.units[i]!.est);
  });

  it("keeps an A cell's repetitions on one host, and confirms the cell there", () => {
    for (const cell of ["A|python|real", "A|python|fixtures", "A|sql|real"]) {
      const hosts = new Set(plan.units.filter((u) => u.group === cell).map((u) => u.host));
      expect(hosts.size).toBe(1);
      const [, pack, corpus] = cell.split("|");
      expect(plan.confirm.find((c) => c.pack === pack && c.corpus === corpus)?.host).toBe([...hosts][0]);
    }
  });

  it("balances the hosts", () => {
    const [h1, h2] = [plan.load["build-1"]!, plan.load["build-2"]!];
    expect(Math.max(h1, h2) / Math.min(h1, h2)).toBeLessThan(1.25);
  });

  it("refuses a plan without hosts or workers", () => {
    expect(() => planFanout([sql], [], ["0-3"], options)).toThrow();
  });
});

describe("claims", () => {
  it("names a unit's claim directory safely and uniquely", () => {
    expect(claimName("B|python|real|003")).toBe("B_python_real_003");
    expect(claimName("A-confirm|python|real")).toBe("A-confirm_python_real");
  });
});

describe("merging the hosts' runs", () => {
  let root = "";
  afterEach(() => removeFolderSync(root));

  it("concatenates a cell's logs from every host, once each line, and copies the other files", () => {
    root = mkdtempSync(join(tmpdir(), "fanout-merge-"));
    const host = (name: string, files: Record<string, string>): string => {
      for (const [path, text] of Object.entries(files)) {
        mkdirSync(join(root, name, path, ".."), { recursive: true });
        writeFileSync(join(root, name, path), text);
      }
      return join(root, name);
    };
    const h1 = host("h1", { "python/real/one-rule.jsonl": '{"tag":"a"}\n{"tag":"b"}\n', "python/one-file/files.json": "[1]", "python/real/main.findings.json": "{}" });
    // h2's last line is still being written: no newline yet.
    const h2 = host("h2", { "python/real/one-rule.jsonl": '{"tag":"b"}\n{"tag":"c"}\n{"tag":"d"', "python/one-file/files.json": "[1]", "python/real/one-rule.findings/main.r1.json": "{}" });
    const into = join(root, "merged");
    expect(mergeTrees([h1, h2], into)).toEqual([]);
    expect(readFileSync(join(into, "python/real/one-rule.jsonl"), "utf8")).toBe('{"tag":"a"}\n{"tag":"b"}\n{"tag":"c"}\n');
    expect(readFileSync(join(into, "python/real/one-rule.findings/main.r1.json"), "utf8")).toBe("{}");
    // Merged again, nothing doubles.
    mergeTrees([h1, h2], into);
    expect(readFileSync(join(into, "python/real/one-rule.jsonl"), "utf8").split("\n").filter((l) => l !== "")).toHaveLength(3);
  });

  it("reports a file that differs between hosts, keeping the first host's", () => {
    root = mkdtempSync(join(tmpdir(), "fanout-merge-"));
    mkdirSync(join(root, "h1"), { recursive: true });
    mkdirSync(join(root, "h2"), { recursive: true });
    writeFileSync(join(root, "h1", "files.json"), "[1]");
    writeFileSync(join(root, "h2", "files.json"), "[2]");
    expect(mergeTrees([join(root, "h1"), join(root, "h2")], join(root, "m"))).toEqual(["files.json"]);
    expect(readFileSync(join(root, "m", "files.json"), "utf8")).toBe("[1]");
  });
});

describe("the remote shell", () => {
  it("is plain ssh unless --ssh or SHIPBAR_SSH names another command, --ssh first", () => {
    expect(remoteShell(undefined, {})).toEqual(["ssh"]);
    expect(remoteShell(undefined, { SHIPBAR_SSH: "  my-wrapper  ssh " })).toEqual(["my-wrapper", "ssh"]);
    expect(remoteShell("ssh -p 2222", { SHIPBAR_SSH: "my-wrapper ssh" })).toEqual(["ssh", "-p", "2222"]);
    expect(() => remoteShell(" ", {})).toThrow(/empty/);
  });

  it("starts a host script detached: its own session, no hangup, output to a file, the shell back at once", () => {
    expect(detached("work/run1", "host.sh", ["build-1", "it's"])).toBe(`cd work/run1 && (setsid nohup bash -l fanout/host.sh 'build-1' 'it'\\''s' > fanout/host.sh.out 2>&1 < /dev/null &)`);
  });
});