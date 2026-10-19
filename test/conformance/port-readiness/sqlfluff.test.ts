import assert from "node:assert/strict";
import { test } from "vitest";

import { sqlfluffSpan, violationSpan } from "../../../conformance/port-readiness/span-units/sqlfluff.js";

test("sqlfluffSpan counts characters, breaks lines as SQLFluff does, and keeps no end", () => {
  const at = (source: string, text: string) => {
    const start = Buffer.byteLength(source.slice(0, source.indexOf(text)), "utf8");
    return sqlfluffSpan(source, start, start + Buffer.byteLength(text, "utf8"));
  };
  // SQLFluff 4.4.0 (postgres, CV02) reports each `ifnull` below at these 1-based lines and positions.
  assert.deepEqual(at("SELECT '𝐁é', ifnull(a, 1) FROM t;\n", "ifnull"), { startLine: 0, startCol: 13, endLine: null, endCol: null });
  assert.deepEqual(at("SELECT 1;\r\nSELECT\tifnull(a, 1) FROM t;\n", "ifnull"), { startLine: 1, startCol: 7, endLine: null, endCol: null });
  assert.deepEqual(at("SELECT 1;\rSELECT ifnull(a, 1) FROM t;\n", "ifnull"), { startLine: 1, startCol: 7, endLine: null, endCol: null });
  assert.deepEqual(at("SELECT 1;\r\rSELECT ifnull(a, 1) FROM t;\n", "ifnull"), { startLine: 2, startCol: 7, endLine: null, endCol: null });
});

test("violationSpan makes SQLFluff's 1-based line_no and line_pos 0-based, with no end", () => {
  assert.deepEqual(violationSpan(2, 8), { startLine: 1, startCol: 7, endLine: null, endCol: null });
});
