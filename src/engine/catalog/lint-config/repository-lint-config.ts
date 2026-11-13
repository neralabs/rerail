/**
 * A repository's own lint configuration, read at set-up so the ports of the packs being set up run the rules the
 * repository's linters run: the same rules on, the same rules off. Each linter's reader (`linter-readers.ts`) reads its
 * linter's config at the root as data, never running it, and gives a verdict per port; `settleVerdicts`
 * (`linter-config.ts`) settles them into rule settings.
 */
import type { RuleCatalog } from "../repository-model.js";
import { portsOf, portsLinter, settleVerdicts, type LinterNote, type PortVerdict, type RepositoryLintConfig } from "./linter-config.js";
import { LINTER_READERS } from "./linter-readers.js";

/**
 * Settings for the rules of packs `packIds` (in `ruleCatalog`) from the lint configs at `root`, with notes on what could
 * not be used. Notes about a linter none of these rules ports are left out.
 */
export function readRepositoryLintConfig(root: string, ruleCatalog: RuleCatalog, packIds: readonly string[]): RepositoryLintConfig {
  const ports = portsOf(ruleCatalog, packIds);
  const fileNotes: LinterNote[] = [];
  const configs = LINTER_READERS.flatMap(reader => {
    const config = reader.read(root, fileNotes);
    return config === undefined ? [] : [config];
  });
  const ported = new Set(LINTER_READERS.filter(reader => ports.some(port => portsLinter(port, reader.kinds))).map(reader => reader.linter));
  const notes = fileNotes.filter(note => ported.has(note.linter)).map(note => note.text);
  const settings = ports.flatMap(port => {
    const verdicts = configs.flatMap((config): PortVerdict[] => {
      const verdict = config.verdict(port, ruleCatalog);
      return verdict === undefined ? [] : [verdict];
    });
    return settleVerdicts(port, verdicts, notes);
  });
  return { settings, notes };
}
