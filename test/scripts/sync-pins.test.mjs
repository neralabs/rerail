// scripts/deps/sync-pins.mjs: the files that pin a version in step with package.json, and bringing them back in step.

import { describe, expect, it } from "vitest";
import {
  ompHostPin,
  pinDrift,
  withAstGrepReference,
  withOmpDependencies,
  withOmpHostPin,
} from "../../scripts/deps/sync-pins.mjs";

const hex = (c) => c.repeat(64);
const ompHost = (version, digests) =>
  [
    'import { createHash } from "node:crypto";',
    "",
    `const version = "${version}";`,
    "// Published release digests, pinned with the SDK rather than trusted at download time.",
    "const digests = {",
    ...Object.entries(digests).map(([asset, digest]) => `  "${asset}": "${digest}",`),
    "};",
    "",
    "function confinedPath() {}",
    "",
  ].join("\n");
const test = (version) => `const cli = "ast-grep";\nconst UPSTREAM_VERSION = "${version}";\n`;
const files = ({ sdk = "18.12.2", cli = "0.51.0", host = sdk, playground = sdk, reference = cli } = {}) => ({
  manifest: { devDependencies: { "@oh-my-pi/pi-coding-agent": sdk, "@ast-grep/cli": cli } },
  ompHost: ompHost(host, { "omp-darwin-arm64": hex("a") }),
  playground: { dependencies: { "@oh-my-pi/pi-ai": playground, "@oh-my-pi/pi-catalog": playground, zod: "^4.4.3" } },
  differentialTest: test(reference),
});

describe("pinDrift", () => {
  it("passes when every pin follows package.json", () => {
    expect(pinDrift(files())).toEqual([]);
  });

  it("names each pin a bump left behind: the OMP host, each playground SDK package, the ast-grep reference", () => {
    expect(pinDrift(files({ host: "18.0.4", playground: "18.0.4", reference: "0.45.3" }))).toEqual([
      { file: "scripts/host/install-omp-host.mjs", pin: "OMP host", has: "18.0.4", wants: "18.12.2" },
      { file: "playground/package.json", pin: "@oh-my-pi/pi-ai", has: "18.0.4", wants: "18.12.2" },
      { file: "playground/package.json", pin: "@oh-my-pi/pi-catalog", has: "18.0.4", wants: "18.12.2" },
      {
        file: "test/engine/code-rule/code-rule-dsl-upstream-differential.test.ts",
        pin: "ast-grep reference",
        has: "0.45.3",
        wants: "0.51.0",
      },
    ]);
  });

  it("refuses a range in package.json, which no pin can follow", () => {
    expect(() => pinDrift(files({ sdk: "^18.12.2" }))).toThrow(/must be an exact version/u);
  });
});

describe("withOmpHostPin", () => {
  const pinned = { "omp-darwin-arm64": hex("a"), "omp-linux-x64": hex("b") };

  it("sets the version and each pinned asset's digest, keeps the asset set and everything else", () => {
    const next = withOmpHostPin(ompHost("18.0.4", pinned), "18.12.2", {
      "omp-darwin-arm64": hex("c"),
      "omp-linux-x64": hex("d"),
      "omp-windows-arm64.exe": hex("e"),
    });
    expect(next).toBe(ompHost("18.12.2", { "omp-darwin-arm64": hex("c"), "omp-linux-x64": hex("d") }));
    expect(ompHostPin(next)).toEqual({ version: "18.12.2", digests: { "omp-darwin-arm64": hex("c"), "omp-linux-x64": hex("d") } });
  });

  it("refuses a release with no digest for a pinned asset, rather than pinning less", () => {
    expect(() => withOmpHostPin(ompHost("18.0.4", pinned), "18.12.2", { "omp-darwin-arm64": hex("c") })).toThrow(
      "OMP 18.12.2's release has no SHA-256 digest for omp-linux-x64",
    );
  });
});

describe("withOmpDependencies and withAstGrepReference", () => {
  it("moves every @oh-my-pi/* dependency and nothing else", () => {
    const manifest = { name: "playground", dependencies: { "@oh-my-pi/pi-ai": "18.0.4", zod: "^4.4.3" }, devDependencies: { "@oh-my-pi/pi-catalog": "18.0.4" } };
    expect(withOmpDependencies(manifest, "18.12.2")).toEqual({
      name: "playground",
      dependencies: { "@oh-my-pi/pi-ai": "18.12.2", zod: "^4.4.3" },
      devDependencies: { "@oh-my-pi/pi-catalog": "18.12.2" },
    });
  });

  it("sets the differential test's reference version", () => {
    expect(withAstGrepReference(test("0.45.3"), "0.51.0")).toBe(test("0.51.0"));
  });
});
