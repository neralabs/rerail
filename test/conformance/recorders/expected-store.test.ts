/**
 * The answer store's client: which account it reaches, and that a blob the store lacks is the only thing read as
 * absent. A store that cannot be read (no login, no network, no permission) must fail, or CI would generate and judge
 * answers a pull request was meant to take from main.
 */
import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import { test } from "vitest";

import { downloadExpected, expectedStore, MAIN_CONTAINER, mainBlobName, prBlobName, latestBlobName, type ExpectedStore } from "../../../conformance/port-readiness/recording/expected-store.js";
import type { CommandResult, CommandRunner } from "../../../conformance/port-readiness/process.js";
import { encodeExpected } from "../../../conformance/port-readiness/recording/expected.js";

const KEY = `demo/bandit/1.9.4/${"e".repeat(64)}`;

/** A runner answering `az storage blob <command>` from `answer`, recording every argv. */
function fakeAz(answer: (command: string, argv: readonly string[]) => CommandResult): { runner: CommandRunner; calls: string[][] } {
  const calls: string[][] = [];
  return {
    calls,
    runner: {
      async run(argv) {
        calls.push([...argv]);
        return answer(argv[3]!, argv);
      },
    },
  };
}

test("the store is the connection string's when one is set, else the signed-in identity's on the named account, else none", () => {
  const { runner } = fakeAz(() => ({ exitCode: 0, stdout: "", stderr: "" }));
  const azurite = expectedStore(runner, { AZURE_STORAGE_CONNECTION_STRING: "UseDevelopmentStorage=true", RERAIL_EXPECTED_ACCOUNT: "rerail" });
  assert.deepEqual("auth" in azurite && azurite.auth, ["--connection-string", "UseDevelopmentStorage=true"]);
  const account = expectedStore(runner, { RERAIL_EXPECTED_ACCOUNT: "rerail" });
  assert.deepEqual("auth" in account && account.auth, ["--auth-mode", "login", "--account-name", "rerail"]);
  // A fork's job has no credentials and leaves the account empty: no store, and the generator only generates.
  assert.ok("unavailable" in expectedStore(runner, { RERAIL_EXPECTED_ACCOUNT: "" }));
  assert.ok("unavailable" in expectedStore(runner, {}));
  assert.equal(mainBlobName(KEY), `${KEY}/expected.json.gz`);
  assert.equal(prBlobName(12, KEY), `pr-12/${KEY}/expected.json.gz`);
  assert.equal(latestBlobName("demo", "bandit"), "latest/demo/bandit.json");
});

test("only a blob the store says it lacks is absent; a failing store throws, its connection string masked", async () => {
  const absent = fakeAz(() => ({ exitCode: 0, stdout: `{"exists": false}`, stderr: "" }));
  const store: ExpectedStore = { runner: absent.runner, auth: ["--connection-string", "secret-connection"], secrets: ["secret-connection"] };
  assert.equal(await downloadExpected(store, MAIN_CONTAINER, mainBlobName(KEY), KEY), null);
  assert.deepEqual(absent.calls[0]!.slice(0, 8), ["az", "storage", "blob", "exists", "--container-name", MAIN_CONTAINER, "--name", mainBlobName(KEY)]);

  const failing = fakeAz(() => ({ exitCode: 1, stdout: "", stderr: "ERROR: Server failed to authenticate the request. secret-connection" }));
  await assert.rejects(downloadExpected({ ...store, runner: failing.runner }, MAIN_CONTAINER, mainBlobName(KEY), KEY), (error: Error) => {
    assert.match(error.message, /az storage blob exists .* exited 1: ERROR: Server failed to authenticate the request\. \*\*\*/u);
    assert.doesNotMatch(error.message, /secret-connection/u);
    return true;
  });
});

test("a downloaded blob is checked against the sha256 its metadata carries and against the key asked for", async () => {
  const encoded = encodeExpected("demo", "bandit", KEY, new Map([["upstream.json", "{}"]]));
  // az writes a download to the --file it is given; the metadata carries `sha256` unless it is undefined.
  const store = (sha256: string | undefined): ExpectedStore => ({
    runner: fakeAz((command, argv) => {
      if (command === "exists") return { exitCode: 0, stdout: `{"exists": true}`, stderr: "" };
      if (command === "download") writeFileSync(argv[argv.indexOf("--file") + 1]!, encoded.bytes);
      if (command === "metadata") return { exitCode: 0, stdout: JSON.stringify(sha256 === undefined ? { commit: "c" } : { sha256, commit: "c", run: "local" }), stderr: "" };
      return { exitCode: 0, stdout: "", stderr: "" };
    }).runner,
    auth: [],
    secrets: [],
  });

  const found = await downloadExpected(store(encoded.sha256), MAIN_CONTAINER, mainBlobName(KEY), KEY);
  assert.deepEqual(found?.files, new Map([["upstream.json", "{}"]]));
  assert.deepEqual(found?.provenance, { commit: "c", run: "local" });
  await assert.rejects(downloadExpected(store("0".repeat(64)), MAIN_CONTAINER, mainBlobName(KEY), KEY), /the store says 000000000000/u);
  await assert.rejects(downloadExpected(store(undefined), MAIN_CONTAINER, mainBlobName(KEY), KEY), /carries no sha256 metadata/u);
  const other = `demo/bandit/1.9.4/${"f".repeat(64)}`;
  await assert.rejects(downloadExpected(store(encoded.sha256), MAIN_CONTAINER, mainBlobName(other), other), /are those for/u);
});
