import { describe, expect, it } from "vitest";
import { adaptCheck, outcomeOf, planSetup } from "../../../benchmark/merge-assist-resolve.js";

describe("planSetup", () => {
  it("splits a pair's setup into installs and named checks, dropping a trailing platform note", () => {
    expect(planSetup("uv venv -q .venv && uv pip install -q -p .venv -e '.[dev]' mypy && .venv/bin/python -m pytest testing/test_a.py -q && .venv/bin/python -m mypy src/_pytest/assertion")).toEqual({
      setup: "uv venv -q .venv && uv pip install -q -p .venv -e '.[dev]' mypy",
      checks: [
        { name: "tests", kind: "tests", run: ".venv/bin/python -m pytest testing/test_a.py -q" },
        { name: "typecheck", kind: "typecheck", run: ".venv/bin/python -m mypy src/_pytest/assertion" },
      ],
    });
    expect(planSetup("RUSTFLAGS='--cfg x' cargo check -p tokio --tests && cargo test -p tokio --test fs && cargo test -p tokio --lib time (Linux)").checks.map(check => [check.name, check.run])).toEqual([
      ["typecheck", "RUSTFLAGS='--cfg x' cargo check -p tokio --tests"],
      ["tests", "cargo test -p tokio --test fs"],
      ["tests-2", "cargo test -p tokio --lib time"],
    ]);
  });
});

describe("adaptCheck", () => {
  const paths = new Set(["tokio/tests/fs.rs", "testing/test_a.py", "caddyconfig/caddyfile/lexer.go"]);

  it("leaves out test targets and paths a side does not have, and drops a check with none left", () => {
    expect(adaptCheck({ name: "tests", kind: "tests", run: "cargo test -p tokio --test fs --test fs_uring" }, paths)).toEqual({ check: { name: "tests", kind: "tests", run: "cargo test -p tokio --test fs" }, dropped: ["--test fs_uring"] });
    expect(adaptCheck({ name: "tests", kind: "tests", run: "cargo test -p tokio --test fs_uring" }, paths)).toEqual({ check: null, dropped: ["--test fs_uring"] });
    expect(adaptCheck({ name: "tests", kind: "tests", run: "pytest testing/test_a.py testing/test_b.py -q" }, paths).check?.run).toBe("pytest testing/test_a.py -q");
    expect(adaptCheck({ name: "tests", kind: "tests", run: "go test ./caddyconfig/caddyfile/" }, paths).dropped).toEqual([]);
    expect(adaptCheck({ name: "typecheck", kind: "typecheck", run: "go vet ./missing/" }, paths).check?.run).toBe("go vet ./missing/");
    expect(adaptCheck({ name: "tests", kind: "tests", run: "X=1 .venv/bin/python -m pytest testing/test_a.py -q" }, paths)).toEqual({ check: { name: "tests", kind: "tests", run: "X=1 .venv/bin/python -m pytest testing/test_a.py -q" }, dropped: [] });
  });
});

describe("outcomeOf", () => {
  it("counts escalation as correct only when neither design was adopted", () => {
    expect(outcomeOf("first", "first")).toBe("correct");
    expect(outcomeOf("second", "first")).toBe("wrong");
    expect(outcomeOf(null, "second")).toBe("escalated");
    expect(outcomeOf(null, "neither")).toBe("correct");
    expect(outcomeOf("first", "neither")).toBe("wrong");
  });
});
