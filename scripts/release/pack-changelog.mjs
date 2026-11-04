// Each released pack version's CHANGELOG.md section (docs/pack-format.md, "Release"), in the format of core's
// (docs/release.md, "Release notes"): a lead, then the changes under Keep a Changelog's headings. Which rules a pack
// ships, and how each starts, is read from its pack.json rather than written in changesets: changesets pile up between
// releases and each one counts against the main it was written on, so their counts and "ships"/"back to candidates"
// lines contradicted one another in a release's notes. The Release pull request (release-version.mjs) compares the
// pack's rules at its last release with the rules it releases now and writes what changed for the people who upgrade;
// the changesets' summaries follow, each under the change type its file name opens with, saying what changed in a
// rule's behaviour. changesets itself writes no changelog (`.changeset/config.json`, `"changelog": false`).
import { changeTypeSections } from "./release-notes.mjs";

/** Above this many rules, a new-rule line counts them by group instead of naming each. */
export const NAMED_RULES = 12;

/** How a rule starts in a repository that adds the pack, in the order the notes list them: on in a mode, or off. */
const START = { steer: "in Steer", log: "in Log", block: "in Block", off: "off" };

/**
 * A pack's rules as its release notes compare them, from its pack.json (`pack`) and `documentMode(entry)`, the mode
 * of the rule document an entry of pack.json's `rules` names: `{ id, group, start }`, `start` `off` for a rule the
 * pack starts off (`on: false`), else the mode it starts in: pack.json's `mode`, or its document's.
 */
export function packRules(pack, documentMode) {
  return (pack.rules ?? []).map((rule) => ({ id: rule.id, group: rule.group ?? "", start: rule.on === false ? "off" : (rule.mode ?? documentMode(rule)) }));
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

/** `a`, `a and b`, `a, b and c`. */
const list = (items) => (items.length < 2 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`);
const code = (id) => `\`${id}\``;
const rules = (count) => `${count} rule${count === 1 ? "" : "s"}`;
/** The starts present among `rules`, in START's order. */
const startsOf = (group) => Object.keys(START).filter((start) => group.some((rule) => rule.start === start));
/** `items` in order of first appearance, counted: `[[item, count], …]`. */
const tally = (items) => [...items.reduce((counts, item) => counts.set(item, (counts.get(item) ?? 0) + 1), new Map())];

/** The new-rule line: each rule by id under how it starts, or above NAMED_RULES, counts by start and by group. */
function addedLine(added) {
  const starts = startsOf(added);
  if (added.length <= NAMED_RULES) {
    const parts = starts.map((start) => {
      const ids = added.filter((rule) => rule.start === start).map((rule) => code(rule.id));
      return `${list(ids)} start${ids.length === 1 ? "s" : ""} ${START[start]}`;
    });
    return `- ${added.length === 1 ? "New rule" : `${added.length} new rules`}: ${parts.join("; ")}.`;
  }
  const byStart = starts.map((start, index) => `${added.filter((rule) => rule.start === start).length}${index === 0 ? " start" : ""} ${START[start]}`);
  const groups = tally(added.map((rule) => rule.group).filter((group) => group !== ""));
  const from = groups.length === 1 ? ` from ${groups[0][0]}` : groups.length > 1 ? ` (${list(groups.map(([group, count]) => `${count} from ${group}`))})` : "";
  return `- ${added.length} new rules${from}: ${list(byStart)}.`;
}

/** The sentence saying how many of a pack's `total` rules start `on`. */
function ruleCount(total, on) {
  if (total === 0) return "Ships no rules yet.";
  if (on > 0 && on < total) return `${rules(total)}, ${on} of them on by default.`;
  const all = total === 1 ? "" : "all ";
  return `${rules(total)}, ${all}${on === 0 ? "off" : "on"} by default.`;
}

/** The entry for the rules of `restarted` that now start `start`, or null with none. */
function restartedEntry(restarted, start) {
  const ids = restarted.filter((rule) => rule.start === start).map((rule) => code(rule.id));
  return ids.length === 0 ? null : { type: "Changed", entry: `- Now start${ids.length === 1 ? "s" : ""} ${START[start]}: ${list(ids)}.` };
}

/**
 * What `changes` (packRuleChanges) says in a released section: `{ lead, entries }`, the lead the pack's rule count, the
 * entries `{ type, entry }`, one line each: its new rules under Added, the rules it no longer ships under Removed and
 * those that start differently under Changed, each rule by id but for many new ones (addedLine).
 */
export function packRuleNotes(changes) {
  const added = changes.added.length > 0 ? [{ type: "Added", entry: addedLine(changes.added) }] : [];
  const removed = changes.removed.length > 0 ? [{ type: "Removed", entry: `- No longer shipped: ${list(changes.removed.map((rule) => code(rule.id)))}.` }] : [];
  const restarted = Object.keys(START).map((start) => restartedEntry(changes.restarted, start)).filter((entry) => entry !== null);
  return { lead: ruleCount(changes.total, changes.on), entries: [...added, ...removed, ...restarted] };
}

/** What every pack's CHANGELOG.md says of itself, as core's does: the format, and where it differs from Keep a Changelog. */
export const PACK_CHANGELOG_INTRO = [
  "Every notable change to this rule pack, latest first. Each release says how many rules the pack ships, then lists its changes under Added, Changed, Deprecated, Removed, Fixed and Security, as in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/); releases from before the pack's notes took that format list them without those headings. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are [changesets](../../.changeset/README.md), so there is no `## Unreleased`.",
  "Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. Below 1.0.0 every release is a patch, and a patch can add, change or remove rules, so read the notes before you update.",
].join("\n\n");

/**
 * `changelog` (a pack's CHANGELOG.md, `""` for a pack with none yet) with `name`'s `version` released on `date`: the
 * title and PACK_CHANGELOG_INTRO, then a new section of `notes`' lead and, under their change types, `notes`' entries
 * (packRuleNotes) followed by `written`, the changesets' entries (changesetNotes), then the releases before it. A
 * section with no entry says its rules did not change. Throws on an entry with no type, which would drop out of the notes.
 */
export function releasedPackChangelog(changelog, name, version, date, notes, written) {
  const entries = [...notes.entries, ...written];
  const untyped = entries.find(({ type }) => type === null);
  if (untyped !== undefined) throw new Error(`a changeset has no change type: ${untyped.entry}`);
  const sections = changeTypeSections(entries);
  const body = [sections.length > 0 ? notes.lead : `${notes.lead} No change to its rules.`, ...sections].join("\n\n");
  const first = changelog.search(/^## /mu);
  const history = first === -1 ? "" : `\n${changelog.slice(first)}`;
  return `# ${name}\n\n${PACK_CHANGELOG_INTRO}\n\n## ${version} (${date})\n\n${body}\n${history}`;
}
