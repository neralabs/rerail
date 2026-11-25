#!/usr/bin/env bash
# Runs inside the Rerail image (Dockerfile), in a checkout of this repository with at least two commits: what the
# reviewer's Rerail module does on a pull request here (neralabs/reviewer, modules/rerail/run.ts), so `:next` moves
# only to a Rerail that can do it. Approves each `path` pack .rerail/packs.json lists and each rule in .rerail/rules/,
# then scans the last commit's change with the rules and packs at HEAD (docs/headless-scan.md, "In CI"). Fails unless
# the scan completed and ran every pack the file lists: a Rerail leaves out a pack whose core range or fields it can't
# take.
#
#   docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$PWD:/repo" -w /repo <image> \
#     bash scripts/release/image/smoke.sh
set -euo pipefail

export RERAIL_EXPERIMENTAL_HEADLESS_SCAN=1
export RERAIL_HOME="${RERAIL_HOME:-$(mktemp -d)}"
repo="$(pwd)"
rerail --version

# The package names of the packs .rerail/packs.json keeps in the repository, and the ids of .rerail/rules/'s rules.
node --input-type=module -e '
  import { existsSync, readdirSync, readFileSync } from "node:fs";
  const read = (path) => JSON.parse(readFileSync(path, "utf8"));
  if (!existsSync(".rerail/packs.json")) {
    console.error("::error::no .rerail/packs.json here: run this in a checkout of the repository the reviewer scans");
    process.exit(1);
  }
  const names = [];
  for (const pack of read(".rerail/packs.json").packs) {
    if (pack.path) names.push(read(`${pack.path}/package.json`).name);
  }
  if (existsSync(".rerail/rules")) {
    for (const file of readdirSync(".rerail/rules")) {
      if (file.endsWith(".json")) names.push(read(`.rerail/rules/${file}`).id);
    }
  }
  console.log(names.join("\n"));
' > "$RERAIL_HOME/approvals"
mapfile -t approvals < "$RERAIL_HOME/approvals"
if [ -n "${approvals[0]:-}" ]; then
  rerail rules approve "${approvals[@]}" --repo "$repo"
fi

# Exit 0 is a completed scan whatever it found; 1 a failed scan, which still prints its report.
scan="$RERAIL_HOME/scan.json"
status=0
rerail scan --scope git-diff --base HEAD~1 --rules-ref HEAD --no-send --json > "$scan" || status=$?
node --input-type=module -e '
  import { readFileSync } from "node:fs";
  const [path, exit] = process.argv.slice(1);
  let scan;
  try {
    scan = JSON.parse(readFileSync(path, "utf8"));
  } catch {
    console.error(`::error::rerail scan exited ${exit} with no report`);
    process.exit(1);
  }
  const ran = scan.packs ?? [];
  const packs = ran.map((p) => `${p.id}@${p.version} (${p.from})`).join(", ");
  const rules = scan.rules?.length ?? 0;
  console.log(`rerail ${scan.tool?.version}: ${scan.status}, ${rules} rules, packs ${packs || "none"}`);
  if (scan.status !== "completed") {
    for (const failure of scan.failures ?? []) console.error(`::error::${failure.message}`);
    process.exit(1);
  }
  // A Rerail that does not read the file runs none of its packs and still completes.
  const listed = JSON.parse(readFileSync(".rerail/packs.json", "utf8")).packs.length;
  if (ran.length !== listed) {
    console.error(`::error::the scan ran ${ran.length} of the ${listed} packs .rerail/packs.json lists`);
    process.exit(1);
  }
' "$scan" "$status"
