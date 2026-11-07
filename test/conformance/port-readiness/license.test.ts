/**
 * Upstream's licence as the gates and the recorder read it: every file at the root named LICENSE or LICENCE, in any
 * letter case and with any extension or suffix, their bytes one after another in name order. A tool-pack keeps the
 * same files under licenses/<project>/, and the license-sha256 check holds them to what the recording carries.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { readGatesManifest } from "../../../conformance/port-readiness/manifest.js";
import { keptLicenseDirectory, licenseBytes, licenseFiles, upstreamLicenseText } from "../../../conformance/port-readiness/recorded-adapter.js";
import { gatedToolPacks } from "../../../conformance/port-readiness/run.js";

function directoryWith(files: Record<string, string>): { directory: string; cleanup: () => void } {
  const directory = mkdtempSync(join(tmpdir(), "port-readiness-license-"));
  for (const [name, text] of Object.entries(files)) writeFileSync(join(directory, name), text);
  return { directory, cleanup: () => rmSync(directory, { recursive: true, force: true }) };
}

test("every LICENSE or LICENCE file is the licence, in any letter case and with any extension or suffix", () => {
  const { directory, cleanup } = directoryWith({
    "licence.txt": "PHP_CodeSniffer\n",
    "license.md": "blade-parser\n",
    LICENSE: "MIT\n",
    "LICENSE-APACHE": "Apache\n",
    "LICENSE-MIT": "MIT\n",
    "UPSTREAM.md": "not a licence\n",
    COPYRIGHT: "not a licence\n",
    UNLICENSE: "not a licence\n",
    "licensed.txt": "not a licence\n",
  });
  try {
    // A directory named like one is not a file of the licence.
    mkdirSync(join(directory, "LICENSES"));
    assert.deepEqual(licenseFiles(directory), ["LICENSE", "LICENSE-APACHE", "LICENSE-MIT", "licence.txt", "license.md"]);
  } finally {
    cleanup();
  }
});

test("a licence of several files is their bytes one after another in name order, and of one file that file's bytes", () => {
  const several = directoryWith({ "LICENSE-MIT": "MIT\n", "LICENSE-APACHE": "Apache\n" });
  const one = directoryWith({ "licence.txt": "PHP_CodeSniffer\r\nlicence\n" });
  const none = directoryWith({ "UPSTREAM.md": "# Upstream\n" });
  try {
    assert.equal(licenseBytes(several.directory)?.toString("utf8"), "Apache\nMIT\n");
    assert.deepEqual(licenseBytes(one.directory), readFileSync(join(one.directory, "licence.txt")));
    assert.equal(upstreamLicenseText(one.directory), "PHP_CodeSniffer\r\nlicence\n");
    assert.equal(licenseBytes(none.directory), null);
    assert.throws(() => upstreamLicenseText(none.directory), /has no LICENSE or LICENCE file at its root/u);
  } finally {
    several.cleanup();
    one.cleanup();
    none.cleanup();
  }
});

test("every gated tool-pack keeps upstream's licence under licenses/<project>/", () => {
  for (const { pack, tool } of gatedToolPacks(PACKAGE_ROOT)) {
    const kept = join(PACKAGE_ROOT, keptLicenseDirectory(readGatesManifest(PACKAGE_ROOT, pack, tool)));
    assert.notEqual(licenseFiles(kept).length, 0, `${pack}/${tool}: ${kept} has no LICENSE or LICENCE file`);
  }
});
