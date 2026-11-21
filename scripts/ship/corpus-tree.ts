// A corpus directory's files and identity, shared by the shippability bar (shipbar.ts) and the translation suite
// (translations.ts): both pin a corpus in corpora.json by the file count and hash hashTree gives.

import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";

/** Every file under `root`, as sorted paths relative to it. */
export function listFiles(root: string): string[] {
  const out: string[] = [];
  const walk = (directory: string): void => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const full = join(directory, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) out.push(relative(root, full).split(sep).join("/"));
    }
  };
  if (existsSync(root)) walk(root);
  return out.sort();
}

/** A tree's identity: its file count, and the sha256 of each sorted path with its content's sha256. */
export function hashTree(root: string): { files: number; sha256: string } {
  const files = listFiles(root);
  const hash = createHash("sha256");
  for (const path of files) hash.update(`${path}\u0000${createHash("sha256").update(readFileSync(join(root, path))).digest("hex")}\n`);
  return { files: files.length, sha256: hash.digest("hex") };
}
