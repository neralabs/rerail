// Diagnostic policy variants. Compare full outputs before drawing performance conclusions.
// Usage: node benchmark/sql-set-candidate-filter.mjs input.policy.json output.policy.json
import { readFileSync, writeFileSync } from "node:fs";

const [input, output] = process.argv.slice(2);
if (!input || !output || input === output) {
  throw new Error("Provide separate input and output policy paths.");
}
const policy = JSON.parse(readFileSync(input, "utf8"));
let guards;
switch (policy.id) {
  case "ruff-b020-loop-variable-overrides-iterator":
    guards = [{ inside: { kind: "for_statement", field: "left", stopBy: "end" } }];
    break;
  case "ruff-b023-function-uses-loop-variable":
    guards = [
      { inside: { matches: "loop", stopBy: "end" } },
      { inside: { any: [{ kind: "function_definition" }, { kind: "lambda" }], stopBy: "end" } },
    ];
    break;
  default:
    throw new Error(`No reviewed candidate filter for ${policy.id}`);
}
if (policy.clauses.length !== 1 || !policy.clauses[0].forbid?.ast?.rule) {
  throw new Error("Expected one AST forbid clause.");
}
const ast = policy.clauses[0].forbid.ast;
const alreadyFiltered = Array.isArray(ast.rule.all) && guards.every((guard, index) =>
  JSON.stringify(ast.rule.all[index]) === JSON.stringify(guard));
if (!alreadyFiltered) ast.rule = { all: [...guards, ast.rule] };
writeFileSync(output, JSON.stringify(policy, null, 2) + "\n");
