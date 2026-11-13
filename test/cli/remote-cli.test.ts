/**
 * `rerail remote`'s dispatch, messages and exit codes, with a fake outbox where the outcome is what the outbox reports.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { runRemoteCommand, type RemoteCliOptions } from "../../src/cli/remote-cli.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";

let home: string;
let output: string;
const stdout = { write: (text: string): boolean => { output += text; return true; } };

beforeEach(() => {
  home = mkdtempSync(join(tmpdir(), "rerail-remote-cli-"));
  output = "";
});

afterEach(() => {
  removeFolderSync(home);
});

type Outbox = NonNullable<RemoteCliOptions["outbox"]>;
const fakeOutbox = (methods: Record<string, unknown>): Outbox => methods as unknown as Outbox;
const run = async (args: readonly string[], extra: Partial<RemoteCliOptions> = {}): Promise<number> => runRemoteCommand(args, { home, stdout, ...extra });

describe("rerail remote", () => {
  it("refuses no action and an unknown one with the usage", async () => {
    await expect(run([])).rejects.toThrow(/^Usage: rerail remote add/u);
    await expect(run(["frobnicate"])).rejects.toThrow(/^Usage: rerail remote add/u);
  });

  it("add: needs a URL before its flags, and exactly one way to give the token", async () => {
    await expect(run(["add"])).rejects.toThrow(/^rerail remote add needs a URL\nUsage: /u);
    await expect(run(["add", "--name", "team"])).rejects.toThrow(/^rerail remote add needs a URL\nUsage: /u);
    await expect(run(["add", "https://example.com", "--nope"])).rejects.toThrow(/^Unknown option: --nope\nUsage: /u);
    await expect(run(["add", "https://example.com"])).rejects.toThrow("Give the token with exactly one of --token-stdin or --token-file <file>");
    await expect(run(["add", "https://example.com", "--token-stdin", "--token-file", "x"])).rejects.toThrow("Give the token with exactly one of --token-stdin or --token-file <file>");
  });

  it("add: takes the token from stdin or a file, and says when it holds event content back", async () => {
    const tokenFile = join(home, "token.txt");
    writeFileSync(tokenFile, "file-token\n");
    expect(await run(["add", "https://one.example.com", "--name", "one", "--token-file", tokenFile])).toBe(0);
    expect(output).toBe("Added remote one (https://one.example.com). Rerail sends this machine's sessions, events, audits and scans to it from now on, starting with what it already holds.\n");
    output = "";
    expect(await run(["add", "https://two.example.com", "--name", "two", "--token-stdin", "--no-content", "--label", "laptop"], { readStdin: async () => "stdin-token\n" })).toBe(0);
    expect(output).toBe("Added remote two (https://two.example.com), without event content. Rerail sends this machine's sessions, events, audits and scans to it from now on, starting with what it already holds.\n");
    output = "";
    expect(await run(["list"])).toBe(0);
    expect(output).toBe("one\thttps://one.example.com\ntwo\thttps://two.example.com\t(no content)\n");
    output = "";
    expect(await run(["list", "--json"])).toBe(0);
    expect((JSON.parse(output) as { remotes: { name: string; label?: string }[] }).remotes.map(remote => [remote.name, remote.label])).toEqual([["one", undefined], ["two", "laptop"]]);
  });

  it("list: says when there is no remote, and names what is excluded", async () => {
    expect(await run(["list"])).toBe(0);
    expect(output).toBe("No remotes: nothing leaves this machine.\n");
    await expect(run(["list", "--nope"])).rejects.toThrow(/^Unknown option: --nope\nUsage: /u);
    output = "";
    writeFileSync(join(home, "remote.json"), JSON.stringify({ remotes: [], exclusions: [{ repositoryKey: "repo-a", from: 0, to: null }] }));
    expect(await run(["list"])).toBe(0);
    expect(output).toBe("No remotes: nothing leaves this machine.\nexcluded\trepo-a\n");
  });

  it("remove: takes one name or URL, forgets the outbox's state, and refuses one it does not know", async () => {
    await expect(run(["remove"])).rejects.toThrow(/^rerail remote remove takes one name or URL\nUsage: /u);
    await expect(run(["remove", "a", "b"])).rejects.toThrow(/^rerail remote remove takes one name or URL\nUsage: /u);
    await expect(run(["remove", "team"])).rejects.toThrow("No remote named team");
    const tokenFile = join(home, "token.txt");
    writeFileSync(tokenFile, "file-token\n");
    await run(["add", "https://one.example.com", "--name", "team", "--token-file", tokenFile]);
    await run(["add", "https://two.example.com", "--name", "other", "--token-file", tokenFile]);
    output = "";
    await run(["list", "--json"]);
    const ids = new Map((JSON.parse(output) as { remotes: { name: string; id: string }[] }).remotes.map(remote => [remote.name, remote.id]));
    expect(ids.get("team")).toEqual(expect.any(String));
    expect(ids.get("team")).not.toBe(ids.get("other"));
    output = "";
    const forgotten: string[] = [];
    expect(await run(["remove", "team"], { outbox: fakeOutbox({ forget: (key: string) => forgotten.push(key) }) })).toBe(0);
    expect(output).toBe("Removed remote team; its token is deleted.\n");
    // The state it forgets is the removed remote's own, under the id its add gave it.
    expect(forgotten).toEqual([ids.get("team")]);
    output = "";
    await run(["list"]);
    expect(output).toBe("other\thttps://two.example.com\n");
  });

  it("status: JSON, none, one line per remote, and exit 1 on a remote still failing", async () => {
    const healthy = { name: "a", url: "https://a/", content: true, lastSuccessAt: null, pendingEvents: 1, queuedScans: 2, nextAttemptAt: null, lastError: null, consecutiveFailures: 0 };
    const recovered = { ...healthy, name: "b", lastError: "old", consecutiveFailures: 0 };
    const failing = { ...healthy, name: "c", lastError: "boom", consecutiveFailures: 2 };
    expect(await run(["status"], { outbox: fakeOutbox({ status: () => [] }) })).toBe(0);
    expect(output).toBe("No remotes: nothing leaves this machine.\n");
    output = "";
    expect(await run(["status", "--json"], { outbox: fakeOutbox({ status: () => [healthy] }) })).toBe(0);
    expect(JSON.parse(output)).toEqual({ remotes: [healthy] });
    output = "";
    expect(await run(["status"], { outbox: fakeOutbox({ status: () => [healthy, recovered] }) })).toBe(0);
    expect(output).toBe("Remote a (https://a/): last sent never, 1 events and 2 scans waiting\nRemote b (https://a/): last sent never, 1 events and 2 scans waiting; last error: old\n");
    expect(await run(["status"], { outbox: fakeOutbox({ status: () => [failing] }) })).toBe(1);
    await expect(run(["status", "--nope"], { outbox: fakeOutbox({ status: () => [] }) })).rejects.toThrow(/^Unknown option: --nope\nUsage: /u);
  });

  it("push: forces every batch, words each outcome, and exits 1 on an error or a missing token", async () => {
    const calls: unknown[] = [];
    const pushing = (results: unknown[]): Outbox => fakeOutbox({ sync: async (options: unknown) => { calls.push(options); return results; } });
    expect(await run(["push"], { outbox: pushing([]) })).toBe(0);
    expect(calls).toEqual([{ force: true, maxBatches: Number.MAX_SAFE_INTEGER }]);
    expect(output).toBe("No remotes: nothing leaves this machine.\n");
    output = "";
    expect(await run(["push", "--json"], { outbox: pushing([{ remote: "a", batches: 0, records: 0 }]) })).toBe(0);
    expect(JSON.parse(output)).toEqual({ results: [{ remote: "a", batches: 0, records: 0 }] });
    output = "";
    expect(await run(["push"], { outbox: pushing([
      { remote: "a", batches: 2, records: 7 },
      { remote: "b", batches: 0, records: 0, skipped: "locked" },
      { remote: "c", batches: 0, records: 0, skipped: "removed" },
    ]) })).toBe(0);
    expect(output).toBe([
      "Remote a: 2 batches, 7 records sent\n",
      "Remote b: skipped: another Rerail process is sending to it\n",
      "Remote c: stopped: it was removed while sending\n",
    ].join(""));
    output = "";
    expect(await run(["push"], { outbox: pushing([{ remote: "a", batches: 0, records: 0, skipped: "no-token" }]) })).toBe(1);
    expect(output).toBe("Remote a: skipped: its token file is missing; remove and add it again\n");
    output = "";
    expect(await run(["push"], { outbox: pushing([{ remote: "a", batches: 1, records: 3, error: "boom" }]) })).toBe(1);
    expect(output).toBe("Remote a: 1 batches, 3 records sent; failed: boom\n");
  });

  it("exclude and include: name the repository, and refuse what they do not take", async () => {
    await expect(run(["exclude", "--nope"])).rejects.toThrow(/^Unknown option: --nope\nUsage: /u);
    await expect(run(["exclude", "--repo", home])).rejects.toThrow(`Rerail has not seen the repository at ${home} yet, and it has no Git remote to name it by`);
    await expect(run(["include", "--nope"])).rejects.toThrow(/^Unknown option: --nope\nUsage: /u);
  });

  it("exclude stops a repository going to any remote, and include lets it go again", async () => {
    const checkout = join(home, "checkout");
    mkdirSync(checkout);
    execFileSync("git", ["init", "--quiet"], { cwd: checkout, stdio: "ignore" });
    execFileSync("git", ["remote", "add", "origin", "git@github.com:acme/app.git"], { cwd: checkout, stdio: "ignore" });
    const key = "remote:github.com/acme/app";
    const listed = async (): Promise<string> => {
      output = "";
      expect(await run(["list"])).toBe(0);
      return output;
    };
    // Including a repository that was never excluded changes nothing.
    expect(await run(["include", "--repo", checkout])).toBe(0);
    expect(output).toBe(`Remotes receive ${key} again from now on; events recorded while it was excluded are not sent.\n`);
    expect(await listed()).toBe("No remotes: nothing leaves this machine.\n");
    output = "";
    expect(await run(["exclude", "--repo", checkout])).toBe(0);
    expect(output).toBe(`No remote receives anything of ${key} from now on; what was sent already stays sent.\n`);
    expect(await listed()).toBe(`No remotes: nothing leaves this machine.\nexcluded\t${key}\n`);
    output = "";
    expect(await run(["include", "--repo", checkout])).toBe(0);
    expect(output).toBe(`Remotes receive ${key} again from now on; events recorded while it was excluded are not sent.\n`);
    expect(await listed()).toBe("No remotes: nothing leaves this machine.\n");
  });
});
