// #10 Each `fixtures/<family>/<feature>` maps to an engine module.
//
// Predicate (owned by ../fixtures.mjs, `fixtureLayout(repo).unmapped`): every directory
// `fixtures/<family>/<feature>/` has `<feature>` equal to an engine module (a top-level `<name>.rs` or
// `<name>/` under native/src/code_rule_dsl/, excluding mod.rs and test_support.rs) or to a documented
// non-engine group (`NON_ENGINE_GROUPS`: `upstream`, the pack test-data layout, and `licenses`).
// Exemption `non-engine-families`: the `host` and `packs` families hold host-adapter and pack-validator
// inputs; their children are areas, not engine features.

import { fixtureLayout } from "../fixtures.mjs";
import { finding, sortFindings } from "../lib.mjs";

export const id = 10;
export const title = "`fixtures/<family>/<feature>` maps to an engine module";
export const target = "100%";

export function check(repo) {
  const { features, unmapped } = fixtureLayout(repo);
  const findings = unmapped.map((dir) =>
    finding(dir, "feature directory names no engine module (native/src/code_rule_dsl/<module>) or documented group"),
  );
  const total = features.length;
  const mapped = total - unmapped.length;
  const percent = total === 0 ? 100 : Math.floor((mapped * 100) / total);
  return { actual: `${percent}% (${mapped}/${total})`, findings: sortFindings(findings) };
}
