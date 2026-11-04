import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { ENTRY_POINTS, PACKAGE_ROOT } from "../../src/engine/package-root.js";

describe("rerail help", () => {
  const home = mkdtempSync(join(tmpdir(), "rerail-usage-"));
  afterAll(() => rmSync(home, { recursive: true, force: true }));

  it("names the running version as the release that leaves out features in internal testing", () => {
    const { version } = JSON.parse(readFileSync(join(PACKAGE_ROOT, "package.json"), "utf8")) as { version: string };
    const usage = execFileSync(process.execPath, [join(PACKAGE_ROOT, ENTRY_POINTS.cli), "--rerail-managed-child", "help"], {
      encoding: "utf8",
      env: { ...process.env, RERAIL_HOME: home },
    });
    expect(usage).toContain(`[Internal testing] Not part of the ${version} release:`);
  });
});
