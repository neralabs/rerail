import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { runDataLayerCommand } from "../../../src/cli/index.js";

it("transfers command help without loading a repository", async () => {
  let output = "";
  expect(await runDataLayerCommand(["--help"], { write(chunk: string) { output += chunk; return true; } })).toBe(0);
  expect(output).toContain("--check");
  expect(output).toContain("--json");
});

it("requires native evidence for every transferable upstream assertion", () => {
  const receipt = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-native-assertions.json", "utf8"));
  expect(receipt.transferableAssertions).toBeDefined();
  expect(receipt.transferableAssertions.sourceCases).toBe(57);
  expect(receipt.transferableAssertions.missing).toEqual([]);
});
