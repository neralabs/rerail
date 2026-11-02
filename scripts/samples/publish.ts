/**
 * Samples staged for publishing: a tool-pack's `publish/samples/` (scripts/samples/pack.ts writes it) holds the files of
 * the samples archive entries it adds or replaces, laid out as in the archive (scripts/samples/format.ts), plus a
 * `REUSE.toml` that licenses the tracked copy and is never archived. The publish workflow's prep
 * (`packFrom`, scripts/s3/pack-fixtures.mjs) packs them through `publishedSamples`.
 */
import { LABELS_SUFFIX, LICENSES_FILE, SAMPLE_ID, sampleLabelsSchema, type SampleLabels } from "./format.js";
import { copyleftReason, recordedLicence } from "./licences.js";
import { sha256 } from "./samples.js";

/** The staged copy's licensing file, beside the samples in `publish/samples/`; prep drops it from the archive. */
export const PUBLISH_REUSE = "REUSE.toml";

interface Entry {
  path: string;
  bytes: Buffer;
}

/**
 * The samples archive entries of the files staged under a tool-pack's `publish/samples/` (`files`, paths relative to
 * it): every file but `REUSE.toml`, once each is checked. A path is `LICENSES.md` or `<sample-id>/<path>` with a
 * SAMPLE_ID; each staged sample is staged whole, its labels (`<source.path>.expected.json`, sampleLabelsSchema) beside
 * its file, whose sha256 is the labels' `source.sha256`; and no sample is taken from a copyleft source
 * (COPYLEFT_SOURCES). A sample's `source.repo` must have a recorded licence (RECORDED_LICENCES) and its declared
 * `source.license` must equal the one recorded at its `source.commit` (recordedLicence), so packFrom's allowlist check
 * on `source.license` judges the recorded value, never a label's own claim. Throws one error naming every problem.
 */
export function publishedSamples(files: readonly Entry[]): Entry[] {
  const problems: string[] = [];
  const entries = files.filter(file => file.path !== PUBLISH_REUSE);
  const byPath = new Map(entries.map(entry => [entry.path, entry]));
  const labelled = new Set<string>();
  for (const { path, bytes } of entries) {
    if (path === LICENSES_FILE) continue;
    const slash = path.indexOf("/");
    const id = path.slice(0, slash);
    if (slash <= 0 || slash === path.length - 1 || !SAMPLE_ID.test(id)) {
      problems.push(`${path}: not ${LICENSES_FILE} or <sample-id>/<path> with a sample id like ${SAMPLE_ID.source}`);
      continue;
    }
    if (!path.endsWith(LABELS_SUFFIX)) continue;
    const parsed = parseLabels(bytes);
    if (typeof parsed === "string") {
      problems.push(`${path}: ${parsed}`);
      continue;
    }
    const filePath = path.slice(0, -LABELS_SUFFIX.length);
    // A context file's own name may end in the suffix only if it is no labels file: labels name their file's path.
    if (filePath !== `${id}/${parsed.source.path}`) {
      problems.push(`${path}: its source.path ${JSON.stringify(parsed.source.path)} is not ${JSON.stringify(filePath.slice(id.length + 1))}`);
      continue;
    }
    labelled.add(id);
    const file = byPath.get(filePath);
    if (file === undefined) problems.push(`${path}: its file ${filePath} is not staged; stage the sample whole`);
    else if (sha256(file.bytes) !== parsed.source.sha256) problems.push(`${filePath}: its sha256 ${sha256(file.bytes)} is not the labels' source.sha256 ${parsed.source.sha256}`);
    const copyleft = copyleftReason(parsed.source.repo);
    if (copyleft !== undefined) {
      problems.push(`${path}: ${parsed.source.repo} is a copyleft source (${copyleft}); no file of it becomes a sample`);
      continue;
    }
    const recorded = recordedLicence(parsed.source.repo, parsed.source.commit);
    if (recorded === undefined) problems.push(`${path}: ${parsed.source.repo} has no recorded licence; read its licence and add it to scripts/samples/licences.ts`);
    else if (parsed.source.license !== recorded) problems.push(`${path}: its source.license ${JSON.stringify(parsed.source.license)} is not ${parsed.source.repo}'s recorded licence at ${parsed.source.commit.slice(0, 12)}, ${JSON.stringify(recorded)}`);
  }
  const unlabelled = [...new Set(entries.flatMap(({ path }) => (path === LICENSES_FILE || !path.includes("/") ? [] : [path.slice(0, path.indexOf("/"))])))]
    .filter(id => SAMPLE_ID.test(id) && !labelled.has(id));
  for (const id of unlabelled) problems.push(`${id}/: no labels file (<path>${LABELS_SUFFIX}) is staged for it; stage the sample whole`);
  if (problems.length > 0) throw new Error(`publish/samples/ is no valid sample set:\n  ${problems.join("\n  ")}`);
  return entries;
}

function parseLabels(bytes: Buffer): SampleLabels | string {
  let json: unknown;
  try {
    json = JSON.parse(bytes.toString("utf8"));
  } catch (error) {
    return `not JSON (${error instanceof Error ? error.message : String(error)})`;
  }
  const result = sampleLabelsSchema.safeParse(json);
  return result.success ? result.data : `not sample labels: ${result.error.issues.map(issue => `${issue.path.join(".") || "(root)"} ${issue.message}`).join("; ")}`;
}
