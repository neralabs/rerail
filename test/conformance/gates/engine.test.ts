import assert from "node:assert/strict";
import { test } from "vitest";

import { withExpando } from "../../../conformance/gates/engine.js";

test("pattern text takes ast-grep's expando exactly where ast-grep's pre_process_pattern puts it", () => {
  assert.equal(withExpando("def f($NAME=$VALUE):", "µ"), "def f(µNAME=µVALUE):");
  assert.equal(withExpando("f($$$ARGS)", "µ"), "f(µµµARGS)");
  assert.equal(withExpando("x = $$$", "µ"), "x = µµµ");
  // A `$` before anything but a capital or an underscore is not a metavariable.
  assert.equal(withExpando("cost = '$5' + $a", "µ"), "cost = '$5' + $a");
  assert.equal(withExpando("$_ + $$B", "µ"), "µ_ + µµB");
});
