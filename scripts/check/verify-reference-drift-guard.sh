#!/usr/bin/env bash
# `npm run check:reference` passes on the committed DSL reference and fails once a grammar
# production drifts from its zod object. Runs on a copy, so the checkout is never edited.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for path in package.json scripts/dsl/code-rule-dsl-reference.mjs src/engine/code-rule/code-rule-schema.ts src/engine/code-rule/code-rule-schema-semantic.ts \
  src/engine/code-rule/code-rule-languages.ts src/engine/code-rule/code-rule-language-registry.generated.ts src/engine/catalog/rule-mode.ts \
  native/src/code_rule_dsl/compile/ast_rule.rs docs/code-rule-dsl-reference.html; do
  mkdir -p "$work/$(dirname "$path")"
  cp "$root/$path" "$work/$path"
done

npm --prefix "$work" run --silent check:reference > /dev/null

reference="$work/docs/code-rule-dsl-reference.html"
node -e '
  const fs = require("node:fs");
  const [path] = process.argv.slice(1);
  const html = fs.readFileSync(path, "utf8");
  const scope = "<b class=\"k\">exclude</b>: [Glob] }";
  if (!html.includes(scope)) throw new Error("the Scope production moved; update the injected drift");
  fs.writeFileSync(path, html.replace(scope, "<b class=\"k\">exclude</b>: [Glob], <b class=\"k\">drift</b>: Str }"));
' "$reference"
if npm --prefix "$work" run --silent check:reference > /dev/null; then
  echo "check:reference accepted a production key that is not in zod" >&2
  exit 1
fi
