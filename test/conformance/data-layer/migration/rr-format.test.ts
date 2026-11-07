import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, test } from "vitest";

const python = process.platform === "win32" ? "python" : "python3";

describe("data-layer .rr authoring contract", () => {
  test("protects the complete DTO comparator and rejects incomplete acceptance evidence", () => {
    expect(() => execFileSync(python, ["-m", "unittest", "discover", "-s", "test/scripts/data_layer_rr", "-p", "test_*.py"], {
      timeout: 30_000, stdio: "pipe",
    })).not.toThrow();
  });

  test("binds imported and repaired .rr sources and forbids additional JSON ports", () => {
    execFileSync(python, ["scripts/dsl/data_layer_rr/migration.py", "inventory"], {
      timeout: 30_000, stdio: "pipe",
    });
    const manifest = JSON.parse(readFileSync("gate/data-layer/packs/data-layer/candidates/rr/migration.json", "utf8")) as {
      rules: { check: string; legacyPath: string }[];
    };
    const authored = ["gate/data-layer/packs/data-layer/rules", "gate/data-layer/packs/data-layer/candidates/rules"]
      .flatMap(directory => readdirSync(directory).filter(file => file.endsWith(".json")).map(file => join(directory, file)))
      .sort();
    expect(authored).toEqual(manifest.rules.map(rule => rule.legacyPath).sort());
  });
});
