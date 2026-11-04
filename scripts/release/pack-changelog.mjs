// The rule summary at the head of each released pack version's CHANGELOG.md section (docs/pack-format.md, "Release").
// Which rules a pack ships, and how each starts, is read from its pack.json rather than written in changesets:
// changesets pile up between releases and each one counts against the main it was written on, so their counts and
// "ships"/"back to candidates" lines contradicted one another in a release's notes. The Release pull request
// (release-version.mjs) compares the pack's rules at its last release with the rules it releases now and writes what
// changed for the people who upgrade; the changesets' summaries follow it, saying what changed in a rule's behaviour.
import { changelogEntries } from "./core.mjs";

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

/**
 * The summary of `changes` (packRuleChanges) a released section opens with: `{ lead, entries }`, the lead the pack's
 * rule count, the entries one line each for its new rules, the rules it no longer ships and those that start
 * differently, each rule by id but for many new ones (addedLine).
 */
export function packRuleNotes(changes) {
  const { total, on } = changes;
  const share = on === total ? (total === 1 ? "on" : "all on") : on === 0 ? (total === 1 ? "off" : "all off") : `${on} of them on`;
  const lead = total === 0 ? "Ships no rules yet." : `${rules(total)}, ${share} by default.`;
  const entries = [];
  if (changes.added.length > 0) entries.push(addedLine(changes.added));
  if (changes.removed.length > 0) entries.push(`- No longer shipped: ${list(changes.removed.map((rule) => code(rule.id)))}.`);
  for (const start of Object.keys(START)) {
    const restarted = changes.restarted.filter((rule) => rule.start === start).map((rule) => code(rule.id));
    if (restarted.length > 0) entries.push(`- Now start${restarted.length === 1 ? "s" : ""} ${START[start]}: ${list(restarted)}.`);
  }
  return { lead, entries };
}

/**
 * `changelog` (a pack's CHANGELOG.md, just written by `changeset version`) with its `## <version>` section dated
 * `date` and led by `notes` (packRuleNotes): the lead, then the rule entries, then the changesets' entries. A section
 * with one `### <Type> Changes` heading loses it: below 1.0.0 every change is a patch and the heading says nothing.
 * One with several (a pack past 1.0.0) keeps them, under the rule entries. A section with no entry of either kind
 * says its rules did not change. Throws when the section is missing.
 */
export function releasedPackSection(changelog, version, date, notes) {
  const lines = changelog.split("\n");
  const start = lines.findIndex((line) => line === `## ${version}`);
  if (start === -1) throw new Error(`the changelog has no undated ## ${version} section to date`);
  const next = lines.findIndex((line, index) => index > start && line.startsWith("## "));
  const end = next === -1 ? lines.length : next;
  const section = lines.slice(start + 1, end).join("\n");
  const typed = section.split("\n").filter((line) => line.startsWith("### ")).length > 1;
  const written = typed ? [] : changelogEntries(section);
  const entries = [...notes.entries, ...written];
  const parts = [notes.lead, ...(entries.length > 0 ? [entries.join("\n")] : typed ? [] : ["- No change to its rules."]), ...(typed ? [section.trim()] : [])];
  const after = lines.slice(end).join("\n");
  return [...lines.slice(0, start), `## ${version} (${date})`, "", parts.join("\n\n"), ...(after === "" ? [""] : ["", after])].join("\n");
}
