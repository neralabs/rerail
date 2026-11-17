import { describe, expect, it } from "vitest";
import { LintConfigFormatError } from "../../../../src/engine/catalog/lint-config/formats/lint-config-formats.js";
import { readRustLintAttributes, type RustLintAttribute } from "../../../../src/engine/catalog/lint-config/rust-lint-attributes.js";

/** One attribute setting `level` for `names`, as `text` writes it, applying always unless `conditional`. */
const attribute = (level: RustLintAttribute["level"], names: string[], text: string, conditional = false): RustLintAttribute => ({ level, names, conditional, text });

/** The message `source` is refused with, as a `LintConfigFormatError`. */
function refusal(source: string): string {
  try {
    readRustLintAttributes(source);
  } catch (error) {
    if (error instanceof LintConfigFormatError) return error.message;
    throw error;
  }
  throw new Error(`${JSON.stringify(source)} was read`);
}

describe("readRustLintAttributes", () => {
  it.each<[string, string, RustLintAttribute[]]>([
    ["a byte order mark", "\uFEFF#![deny(clippy::all)]\n", [attribute("deny", ["clippy::all"], "#![deny(clippy::all)]")]],
    ["a shebang alone, with no line end", "#!/usr/bin/env run-cargo-script", []],
    ["a shebang", "#!/usr/bin/env run-cargo-script\n#![warn(clippy::pedantic)]\n", [attribute("warn", ["clippy::pedantic"], "#![warn(clippy::pedantic)]")]],
    ["`#!` then a comment then `[`, an attribute", "#! // not a shebang\n[allow(clippy::needless_else)]\n", [attribute("allow", ["clippy::needless_else"], "#![allow(clippy::needless_else)]")]],
    ["`#!` then far-off `[`, an attribute", `#!${" ".repeat(100)}[allow(clippy::needless_else)]`, [attribute("allow", ["clippy::needless_else"], "#![allow(clippy::needless_else)]")]],
    [
      "comments, doc comments and nested block comments",
      "//! The crate.\n/** Docs. */\n/* outer /* inner */ still */\n#![forbid(unsafe_code)]\n// The end.",
      [attribute("forbid", ["unsafe_code"], "#![forbid(unsafe_code)]")],
    ],
    [
      "other attributes among them",
      "#![no_std]\n#![doc = \"x\"]\n#![feature(let_chains)]\n#![recursion_limit = \"256\"]\n#![allow(clippy::needless_else)]\n",
      [attribute("allow", ["clippy::needless_else"], "#![allow(clippy::needless_else)]")],
    ],
    [
      "several lints, a reason and a trailing comma",
      "#![expect(\n  clippy::needless_else,\n  warnings,\n  reason = r#\"a \")] reason\"#,\n)]\n",
      [attribute("expect", ["clippy::needless_else", "warnings"], "#![expect( clippy::needless_else, warnings, reason = r#\"a \")] reason\"#, )]")],
    ],
    ["only the attributes before the first item", "use std::fmt;\n#![allow(clippy::needless_else)]\n", []],
    ["not an outer attribute, which starts an item", "#[derive(Debug)]\nstruct S;\n#![allow(clippy::needless_else)]\n", []],
    ["character and lifetime tokens in other attributes", "#![doc = 'x']\n#![x('a, b'\\n')]\n#![deny(warnings)]\n", [attribute("deny", ["warnings"], "#![deny(warnings)]")]],
  ])("reads %s", (_, source, expected) => {
    expect(readRustLintAttributes(source)).toEqual(expected);
  });

  it("takes `cfg_attr(clippy, …)` as always applying and any other `cfg_attr` as conditional", () => {
    const source = [
      "#![cfg_attr(clippy, allow(clippy::needless_else), warn(clippy::pedantic))]",
      "#![cfg_attr(test, deny(warnings))]",
      "#![cfg_attr(not(clippy), deny(clippy::all))]",
      "#![cfg_attr(clippy, cfg_attr(clippy, forbid(clippy::correctness)))]",
      "#![cfg_attr(test, cfg_attr(clippy, allow(clippy::style)))]",
      "#![cfg_attr(clippy, feature(let_chains))]",
      "#![cfg_attr()]",
    ].join("\n");
    expect(readRustLintAttributes(source)).toEqual([
      attribute("allow", ["clippy::needless_else"], "#![cfg_attr(clippy, allow(clippy::needless_else), warn(clippy::pedantic))]"),
      attribute("warn", ["clippy::pedantic"], "#![cfg_attr(clippy, allow(clippy::needless_else), warn(clippy::pedantic))]"),
      attribute("deny", ["warnings"], "#![cfg_attr(test, deny(warnings))]", true),
      attribute("deny", ["clippy::all"], "#![cfg_attr(not(clippy), deny(clippy::all))]", true),
      attribute("forbid", ["clippy::correctness"], "#![cfg_attr(clippy, cfg_attr(clippy, forbid(clippy::correctness)))]"),
      attribute("allow", ["clippy::style"], "#![cfg_attr(test, cfg_attr(clippy, allow(clippy::style)))]", true),
    ]);
  });

  it.each<[string, string, string]>([
    ["`#!` not opening an attribute", "#![allow(warnings)]\n#!x\n", "line 2: expected [ after #!"],
    ["an unclosed block comment", "#![allow(warnings)]\n/* open /* nested */\n", "line 2: a block comment is not closed"],
    ["an unclosed string", "#![allow(warnings, reason = \"open)]\n", "line 1: a literal is not closed"],
    ["an unclosed character", "#![doc = ' ]\n", "line 1: a literal is not closed"],
    ["an unclosed bracket", "#![allow(warnings\n", "line 2: a ( is not closed"],
    ["a mismatched bracket", "#![allow(warnings]\n", "line 1: a ( is not closed"],
    ["brackets nested too deep", `#![${"(".repeat(70)}`, "line 1: brackets nest more than 64 deep"],
    ["a path ending in `::`", "#![allow(clippy::)]\n", "#![allow(clippy::)] names something that is not a lint"],
    ["a string for a lint", "#![deny(\"warnings\")]\n", "#![deny(\"warnings\")] names something that is not a lint"],
    ["two paths in one lint", "#![warn(clippy::all clippy::pedantic)]\n", "#![warn(clippy::all clippy::pedantic)] names something that is not a lint"],
  ])("refuses %s", (_, source, message) => {
    expect(refusal(source)).toBe(message);
  });
});
