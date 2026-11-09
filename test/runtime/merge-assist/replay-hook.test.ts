import { execFile } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import { afterEach, describe, expect, it } from "vitest";
import { leakReasons, withoutSearchPatterns } from "../../../benchmark/merge-assist-replay.js";
import type { ReplayHookConfig, ReplayHookLogEntry } from "../../../benchmark/merge-assist-replay-hook.js";

const run = promisify(execFile);
const HOOK = resolve("benchmark/merge-assist-replay-hook.ts");
const ALIGN = resolve("benchmark/merge-assist-align.ts");

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** One hook call as Codex makes it: the input on stdin, the run's config through `MA_CONTEXT_CONFIG`. */
function hookCall(configPath: string, input: object): Promise<{ stdout: string }> {
  const child = run(process.execPath, ["--import", "tsx", HOOK], { env: { ...process.env, MA_CONTEXT_CONFIG: configPath } });
  child.child.stdin!.end(JSON.stringify(input));
  return child;
}

describe("replay hook", () => {
  it("delivers a unit once when two calls open its area at the same time", async () => {
    const folder = mkdtempSync(join(tmpdir(), "rerail-replay-hook-"));
    roots.push(folder);
    const root = join(folder, "repo");
    mkdirSync(join(root, "src", "store"), { recursive: true });
    writeFileSync(join(root, "src", "store", "a.ts"), "export const a = 1;\n");
    writeFileSync(join(root, "src", "store", "b.ts"), "export const b = 2;\n");
    const config: ReplayHookConfig = {
      arm: "units",
      units: [{ id: "u1", kind: "position", text: "The store owns its cache.", author: "alice", pr: 12, anchors: { files: ["src/store/a.ts"], symbols: [] } }],
      alignModule: ALIGN,
      alignOptions: { cacheDir: join(folder, "align-cache"), noJudge: true },
      prompt: "Change the store.",
      root,
      log: join(folder, "hook.log"),
      state: join(folder, "hook-state.json"),
      refs: [],
      workspaceBase: "",
      conflictsFile: null,
    };
    const configPath = join(folder, "config.json");
    writeFileSync(configPath, JSON.stringify(config));
    writeFileSync(config.log, "");
    const read = (file: string) => ({ hook_event_name: "PostToolUse", tool_name: "Bash", cwd: root, tool_input: { command: `cat src/store/${file}` } });

    const outputs = await Promise.all([hookCall(configPath, read("a.ts")), hookCall(configPath, read("b.ts"))]);

    const entries = readFileSync(config.log, "utf8").split("\n").filter(Boolean).map(line => JSON.parse(line) as ReplayHookLogEntry);
    expect(entries.map(entry => entry.error)).toEqual([undefined, undefined]);
    expect(entries.flatMap(entry => entry.delivered.map(item => item.unitId))).toEqual(["u1"]);
    expect(outputs.filter(output => output.stdout.includes("PR #12 (alice) established: The store owns its cache.")).length).toBe(1);
  }, 60_000);
});

describe("replay leak guard", () => {
  const folder = "/tmp/rerail-replay-run";

  it("does not read a search pattern that names `set` as an environment dump", () => {
    const event = "/bin/bash -lc \"rg -n \\\"Implemented path \\\\(list step 26|Each emitted row set|List step 26|per-document fact producer|canonical rule digest|O41|note t15|O44\\\" docs/final-shape/open-items.md docs/final-shape/05-implementation-plan.md docs/final-shape/decisions/0030-semantic-fact-labels.md docs/final-shape/decisions/README.md && git diff --check && git status --short\"";
    const transcript = "\"rg -n \\\"Implemented path \\\\(list step 26|Each emitted row set|List step 26|per-document fact producer|canonical rule digest|O41|note t15|O44\\\" docs/final-shape/open-items.md docs/final-shape/05-implementation-plan.md docs/final-shape/decisions/0030-semantic-fact-labels.md docs/final-shape/decisions/README.md && git diff --check && git status --short\"";
    expect(leakReasons(event, folder, folder)).not.toContain("environment dump");
    expect(leakReasons(transcript, folder, folder)).not.toContain("environment dump");
    expect(leakReasons("grep -rn 'process.env' src", folder, folder)).not.toContain("environment dump");
  });

  it("flags the tamper probe and other commands that print the environment", () => {
    const tamperEvent = "/bin/bash -lc \"env | grep -i -E 'aws|token|bedrock' | sed -E 's/=.*\"'$/=[REDACTED]/'\"'\"";
    const tamperTranscript = "\"env | grep -i -E 'aws|token|bedrock' | sed -E 's/=.*$/=[REDACTED]/'\"";
    for (const command of [tamperEvent, tamperTranscript, "env", "printenv AWS_REGION", "cd src && set > out.txt", "export -p", "cat /proc/self/environ", "python3 -c \"import os; print(os.environ)\"", "node -e 'console.log(process.env)'"]) {
      expect(leakReasons(command, folder, folder), command).toContain("environment dump");
    }
  });

  it("empties only the quoted arguments of search commands", () => {
    expect(withoutSearchPatterns("rg -n \"a|env|b\" src && echo \"env\"")).toBe("rg -n \"\" src && echo \"env\"");
  });
});
