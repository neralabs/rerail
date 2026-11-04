// Each released pack version's CHANGELOG.md section (docs/pack-format.md, "Release"), in the format of core's
// (docs/release.md, "Release notes"): a lead, then the changes under Keep a Changelog's headings. Which rules a pack
// ships, and how each starts, is read from its pack.json rather than written in changesets: changesets pile up between
// releases and each one counts against the main it was written on, so their counts and "ships"/"back to candidates"
// lines contradicted one another in a release's notes. The Release pull request (release-version.mjs) compares the
// pack's rules at its last release with the rules it releases now and writes one entry per rule for the people who
// upgrade; the changesets' summaries follow, each under the change type its file name opens with, saying what changed
// in a rule's behaviour. changesets itself writes no changelog (`.changeset/config.json`, `"changelog": false`).
import { changeTypeSections } from "./release-notes.mjs";

/** How a rule starts in a repository that adds the pack, in the order the notes list them: on in a mode, or off. */
const START = { steer: "in Steer", log: "in Log", block: "in Block", off: "off" };

/**
 * A pack's rules as its release notes compare them, from its pack.json (`pack`) and `documentOf(entry)`, the rule
 * document an entry of pack.json's `rules` names (undefined where it has none): `{ id, start, description }`, `start`
 * `off` for a rule the pack starts off (`on: false`), else the mode it starts in: pack.json's `mode`, or its document's.
 */
export function packRules(pack, documentOf) {
  return (pack.rules ?? []).map((rule) => {
    const document = documentOf(rule);
    return { id: rule.id, start: rule.on === false ? "off" : (rule.mode ?? document?.mode), description: document?.description };
  });
}

/**
 * What changed between a pack's rules at its last release (`before`, `[]` for a pack never released) and those it
 * releases now (`after`), each as packRules reads them: `{ total, on, added, removed, restarted }`, `added` the rules
 * new to it, `removed` those it no longer ships, and `restarted` each rule in both whose start changed, with `from`.
 */
export function packRuleChanges(before, after) {
  const was = new Map(before.map((rule) => [rule.id, rule]));
  const is = new Set(after.map((rule) => rule.id));
  return {
    total: after.length,
    on: after.filter((rule) => rule.start !== "off").length,
    added: after.filter((rule) => !was.has(rule.id)),
    removed: before.filter((rule) => !is.has(rule.id)),
    restarted: after.filter((rule) => was.has(rule.id) && was.get(rule.id).start !== rule.start).map((rule) => ({ ...rule, from: was.get(rule.id).start })),
  };
}

const code = (id) => `\`${id}\``;
const rules = (count) => `${count} rule${count === 1 ? "" : "s"}`;
/** `group` in the order the notes list rules: by how each starts, in START's order, then by id. */
const byStart = (group) => {
  const starts = Object.keys(START);
  return [...group].sort((a, b) => starts.indexOf(a.start) - starts.indexOf(b.start) || (a.id < b.id ? -1 : 1));
};

/** The sentence saying how many of a pack's `total` rules start `on`. */
function ruleCount(total, on) {
  if (total === 0) return "Ships no rules yet.";
  if (on > 0 && on < total) return `${rules(total)}, ${on} of them on by default.`;
  const all = total === 1 ? "" : "all ";
  return `${rules(total)}, ${all}${on === 0 ? "off" : "on"} by default.`;
}

/** A new rule's entry: its id, how it starts and what it wants. Throws without a description, which pack validation requires. */
function addedEntry(rule) {
  if (rule.description === undefined) throw new Error(`${rule.id}'s document gives it no description`);
  return { type: "Added", entry: `- ${code(rule.id)} (starts ${START[rule.start]}): ${rule.description}` };
}

/**
 * What `changes` (packRuleChanges) says in a released section: `{ lead, entries }`, the lead the pack's rule count, the
 * entries `{ type, entry }`, one per rule: its new rules under Added, the rules it no longer ships under Removed and
 * those that start differently under Changed, each group in the order byStart gives (Removed by id).
 */
export function packRuleNotes(changes) {
  const added = byStart(changes.added).map(addedEntry);
  const removed = byStart(changes.removed.map((rule) => ({ id: rule.id }))).map((rule) => ({ type: "Removed", entry: `- ${code(rule.id)}: no longer shipped.` }));
  const restarted = byStart(changes.restarted).map((rule) => ({ type: "Changed", entry: `- ${code(rule.id)} now starts ${START[rule.start]} (was ${START[rule.from]}).` }));
  return { lead: ruleCount(changes.total, changes.on), entries: [...added, ...removed, ...restarted] };
}

/** The rule an entry opens with (`` - `rule-id` … ``), or null. */
const ruleOf = (entry) => /^- `([^`]+)`/u.exec(entry)?.[1] ?? null;

/**
 * The entries of a released section: `generated` (packRuleNotes), then `written` (the changesets'), less each generated
 * Removed entry whose rule a written one opens with, since that one also says why the rule left.
 */
function mergedEntries(generated, written) {
  const explained = new Set(written.filter(({ type }) => type === "Removed").map(({ entry }) => ruleOf(entry)));
  return [...generated.filter(({ type, entry }) => type !== "Removed" || !explained.has(ruleOf(entry))), ...written];
}

/** What every pack's CHANGELOG.md says of itself, as core's does: the format, and where it differs from Keep a Changelog. */
export const PACK_CHANGELOG_INTRO = [
  "Every notable change to this rule pack, latest first. Each release says how many rules the pack ships, then lists its changes, rule by rule, under Added, Changed, Deprecated, Removed, Fixed and Security, as in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/); releases from before the pack's notes took that format list them without those headings. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are [changesets](../../.changeset/README.md), so there is no `## Unreleased`.",
  "Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. Below 1.0.0 every release is a patch, and a patch can add, change or remove rules, so read the notes before you update.",
].join("\n\n");

/**
 * `changelog` (a pack's CHANGELOG.md, `""` for a pack with none yet) with `name`'s `version` released on `date`: the
 * title and PACK_CHANGELOG_INTRO, then a new section of `notes`' lead and, under their change types, `notes`' entries
 * (packRuleNotes) followed by `written`, the changesets' entries (changesetNotes), then the releases before it. A
 * section with no entry says its rules did not change. Throws on an entry with no type, which would drop out of the notes.
 */
export function releasedPackChangelog(changelog, name, version, date, notes, written) {
  const entries = mergedEntries(notes.entries, written);
  const untyped = entries.find(({ type }) => type === null);
  if (untyped !== undefined) throw new Error(`a changeset has no change type: ${untyped.entry}`);
  const sections = changeTypeSections(entries);
  const body = [sections.length > 0 ? notes.lead : `${notes.lead} No change to its rules.`, ...sections].join("\n\n");
  const first = changelog.search(/^## /mu);
  const history = first === -1 ? "" : `\n${changelog.slice(first)}`;
  return `# ${name}\n\n${PACK_CHANGELOG_INTRO}\n\n## ${version} (${date})\n\n${body}\n${history}`;
}
