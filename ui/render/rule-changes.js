/**
 * What changed in the rules created by you, said after the fact. A rule added or edited from a coding agent's session
 * or a terminal, and approved there, runs as it is set with nothing asked in the dashboard, so the dashboard says so in
 * three places. Overview counts what was added and edited since the notice was last dismissed; a pack's page names its
 * rules whose edited copy runs in their place; a rule's Source says when it was last edited, what it was edited from and
 * what changed. A rule that still waits for approval is not counted: the approval banners and boxes speak for it. The
 * dashboard itself only switches and deletes rules, so every change counted here was made outside it.
 */
import { h, icon } from "../dom.js";
import { andList, fmt, whenLabel } from "../format.js";
import { awaitingApproval, isLog, messageText } from "../derive.js";
import { data, attachedPackOf, packOfRule, rulesArray, titleOf } from "../state.js";

const DAY_MS = 24 * 60 * 60 * 1000;
/** A change older than this is not announced to a browser that has never dismissed the notice. */
const FIRST_LOOK_MS = 7 * DAY_MS;

/* ---------- which rules ---------- */

/** A rule created by you (in the library), as opposed to one from a rule pack. */
const isYours = stored => stored.origin === "user";
/** An edited copy: a copy, among the rules created by you, of another rule (`<id>-copy`). */
export const isEditedCopy = stored => isYours(stored) && Boolean(stored.copiedFromRuleId);
/** A rule created by you that runs as it is set: one that does not wait for approval. */
const runsAsSet = stored => isYours(stored) && !awaitingApproval(stored);
const byTitle = (a, b) => titleOf(a).localeCompare(titleOf(b), undefined, { sensitivity: "base" }) || a.rule.id.localeCompare(b.rule.id);
/** The rule an edited copy was edited from, when this page knows it. */
const originalOf = copy => data.rules.get(copy.copiedFromRuleId);
/** The pack a rule is in. */
const packOf = stored => attachedPackOf(stored) ?? packOfRule(stored);
/** Where a rule lives, by name: its pack, or Created by you. */
const whereOf = stored => packOf(stored)?.name ?? "Created by you";

/**
 * The edited copy of `stored` among `rules` that runs, the newest when there are several; `null` when it has none. A
 * copy that waits for approval is left out: its original keeps running until then, and the approval box says so.
 */
export function editedCopyIn(rules, stored) {
  const copies = rules.filter(other => isEditedCopy(other) && runsAsSet(other) && other.copiedFromRuleId === stored.rule.id);
  return copies.sort((a, b) => (b.addedAt ?? 0) - (a.addedAt ?? 0))[0] ?? null;
}
/** The edited copy of `stored` that this repository runs, if any. */
export const editedCopyOf = stored => (data.loaded ? editedCopyIn(rulesArray(), stored) : null);

/** When a rule created by you was last edited, if that was after it was added; `null` otherwise. */
export function lastEditedAt(stored) {
  if (!isYours(stored) || stored.updatedAt === null) return null;
  return stored.updatedAt > (stored.addedAt ?? 0) ? stored.updatedAt : null;
}

/**
 * The rules created by you that run as set and were added or edited after `since`. Edited: an edited copy of another
 * rule, or a rule of yours edited in place. A rule both added and edited since then counts as added.
 */
export function changedSince(rules, since) {
  const yours = rules.filter(runsAsSet).sort(byTitle);
  const fresh = yours.filter(stored => (stored.addedAt ?? 0) > since);
  const editedInPlace = yours.filter(stored => (stored.addedAt ?? 0) <= since && (lastEditedAt(stored) ?? 0) > since);
  return {
    added: fresh.filter(stored => !isEditedCopy(stored)),
    edited: [...fresh.filter(isEditedCopy), ...editedInPlace].sort(byTitle),
  };
}

/* ---------- Overview: "Your rules changed" ---------- */

const seenKey = () => `rerail-rule-changes-seen:${data.repository?.repositoryKey ?? ""}`;
/** When the notice was dismissed in this visit, which holds when storage is blocked. */
const seenHere = new Map();

/** When this browser last dismissed the notice here; a week ago for one that never has. */
function seenAt() {
  const key = seenKey();
  if (seenHere.has(key)) return seenHere.get(key);
  let stored = 0;
  try { stored = Number(localStorage.getItem(key)) || 0; } catch { /* storage blocked: as never dismissed */ }
  return stored || Date.now() - FIRST_LOOK_MS;
}

/** Dismiss the notice: what changed up to now is seen. */
export function dismissRuleChanges() {
  const key = seenKey(), now = Date.now();
  seenHere.set(key, now);
  try { localStorage.setItem(key, String(now)); } catch { /* private mode: dismissed for this visit */ }
}

/** The rules created by you that run as set and were added or edited since the notice was last dismissed. */
export const ruleChanges = () => (data.loaded ? changedSince(rulesArray(), seenAt()) : { added: [], edited: [] });

/** "2 added and 1 edited". */
export const changeCounts = ({ added, edited }) => [added.length && `${fmt(added.length)} added`, edited.length && `${fmt(edited.length)} edited`]
  .filter(Boolean).join(" and ");

/** "2 set to Steer", or nothing for none. */
const counted = (n, what) => (n === 0 ? [] : [`${fmt(n)} ${what}`]);

/** How the changed rules are set, in a clause: "set to Steer", "set to Log only", or the counts of each. */
export function setToText(rules) {
  const on = rules.filter(stored => stored.enabled), log = on.filter(isLog).length, steer = on.length - log;
  if (steer === rules.length) return "set to Steer";
  if (log === rules.length) return "set to Log only";
  return [...counted(steer, "set to Steer"), ...counted(log, "set to Log only"), ...counted(rules.length - on.length, "off")].join(", ");
}

/** The originals that an edited copy turned off, of the edited rules. */
const originalsOff = edited => edited.filter(isEditedCopy).map(originalOf).filter(original => original && !original.enabled);

/** What the edits did to their originals: turned them off, since each edited copy runs in its original's place. */
export function originalsText(edited, only) {
  const off = originalsOff(edited);
  if (off.length === 0) return "";
  if (edited.length === 1) return only ? ` The original rule has been turned off in ${whereOf(off[0])}.` : ` For the edited rule, the original rule has been turned off in ${whereOf(off[0])}.`;
  return off.length === edited.length ? " For each edited rule, the original rule has been turned off in its pack."
    : ` For ${fmt(off.length)} of the edited rules, the original rule has been turned off in its pack.`;
}

/** A link inside a sentence: a rule's title can be long, so it wraps with the sentence, which a button cannot. */
const inlineLink = (act, target, label) => h("span.textlink.inline", { role: "link", tabindex: "0", "data-act": act, "data-i": target }, label);
const ruleLink = (stored, label) => inlineLink("detail", stored.rule.id, label);
const createdLink = () => inlineLink("nav", "created", "Created by you");
/** Links as "A, B and C". */
const linkList = links => links.flatMap((link, i) => (i === 0 ? [link] : [i === links.length - 1 ? " and " : ", ", link]));

/**
 * Overview's notice for the rules created by you that were added or edited since it was last dismissed: how many,
 * where they are, how they are set, what an edit did to its original, and that any of them can be deleted. It names no
 * rule, so it is the same size for three changes or thirty, and it asks for nothing, so it has no button.
 */
export function ruleChangesNotice() {
  const changes = ruleChanges(), all = [...changes.added, ...changes.edited], one = all.length === 1;
  if (all.length === 0) return null;
  const set = setToText(all);
  return h("div.welcome.changed", { role: "status" }, icon("pencil", 22), h("div", null,
    h("b", null, `Your rules changed: ${changeCounts(changes)}.`),
    h("p", null, one ? "It’s in " : "They’re in ", createdLink(), /\d/u.test(set) ? `: ${set}.` : `, ${set}.`, originalsText(changes.edited, one),
      one ? " You can delete it from its details." : " You can delete any of them from the rule’s details.")),
  h("button.x", { "data-act": "dismissrulechanges", "aria-label": "Dismiss" }, "✕"));
}
export const ruleChangesKey = () => {
  const { added, edited } = ruleChanges();
  return JSON.stringify([[...added, ...edited].map(stored => [stored.rule.id, stored.enabled, isLog(stored)]), originalsOff(edited).length]);
};

/* ---------- a pack's page: a rule here was edited and turned off ---------- */

/** The pack's rules that are off while their edited copy runs in their place, each with that copy. */
function editedIn(pack) {
  if (!data.loaded || !pack) return [];
  return rulesArray().filter(copy => isEditedCopy(copy) && runsAsSet(copy)).map(copy => ({ original: originalOf(copy), copy }))
    .filter(({ original }) => original && packOf(original)?.id === pack.id && !original.enabled)
    .sort((a, b) => byTitle(a.original, b.original));
}

const packSeenKey = packId => `rerail-pack-edits-seen:${data.repository?.repositoryKey ?? ""}:${packId}`;
const packSeenHere = new Map();
/** The edited copies whose notice was dismissed on this pack's page. */
function packSeen(packId) {
  const key = packSeenKey(packId);
  if (packSeenHere.has(key)) return packSeenHere.get(key);
  try { return JSON.parse(localStorage.getItem(key) ?? "[]"); } catch { return []; /* storage blocked, or not a list */ }
}
/** The pack's edits whose notice has not been dismissed. */
function packEdits(pack) {
  const seen = new Set(pack ? packSeen(pack.id) : []);
  return editedIn(pack).filter(({ copy }) => !seen.has(copy.rule.id));
}

/** Dismiss the notice on pack `packId`'s page for the edits it names; a later edit in the pack shows it again. */
export function dismissPackEdits(packId) {
  const key = packSeenKey(packId), seen = [...new Set([...packSeen(packId), ...editedIn({ id: packId }).map(({ copy }) => copy.rule.id)])];
  packSeenHere.set(key, seen);
  try { localStorage.setItem(key, JSON.stringify(seen)); } catch { /* private mode: dismissed for this visit */ }
}

const quoted = stored => `“${titleOf(stored)}”`;
const oneEdit = ({ original, copy }) => [
  h("b", null, "1 rule here was edited and turned off: ", ruleLink(original, quoted(original)), "."),
  h("p", null, ruleLink(copy, "The edited copy"), " runs from ", createdLink(), ". To go back, delete it there and turn this one on again."),
];
const manyEdits = edits => [
  h("b", null, `${fmt(edits.length)} rules here were edited and turned off.`),
  h("p", null, linkList(edits.map(({ original }) => ruleLink(original, quoted(original)))), "."),
  h("p", null, "Their edited copies run from ", createdLink(), ". To go back on one, delete its edited copy there and turn the original on again."),
];

/**
 * A pack's page, when a rule here is off because its edited copy runs in its place: which rule, where the edited copy
 * is, and how to go back. Without it nothing on the page says why the rule is off.
 */
export function packEditsNotice(pack) {
  const edits = packEdits(pack);
  if (edits.length === 0) return null;
  return h("div.welcome.changed", { role: "status" }, icon("pencil", 22),
    h("div", null, edits.length === 1 ? oneEdit(edits[0]) : manyEdits(edits)),
    h("button.x", { "data-act": "dismisspackedits", "data-i": pack.id, "aria-label": "Dismiss" }, "✕"));
}
export const packEditsKey = pack => JSON.stringify([pack?.id ?? null, packEdits(pack).map(({ original, copy }) => [original.rule.id, copy.rule.id])]);

/* ---------- a rule's details: rows under Source ---------- */

const json = x => JSON.stringify(x ?? null);
/** Whether a rule is set to Log only: one that waits for approval is held on Log whatever it is set to, which is no edit. */
const setToLog = stored => isLog(stored) && stored.modeSource?.by !== "awaiting-approval";
/** A clause without its message, to tell a change in what it checks from a change in what it says. */
const withoutMessage = clauses => (clauses ?? []).map(c => ({ ...c, violation: { ...c.violation, message: undefined } }));

/** The sections of a rule's details that an edit changed, in the details' order. */
export function changedParts(was, now) {
  const a = was.rule, b = now.rule;
  return [
    setToLog(was) !== setToLog(now) && "On a violation",
    messageText(was) !== messageText(now) && "What the agent sees",
    (a.description ?? "") !== (b.description ?? "") && "Description",
    json(a.scope) !== json(b.scope) && "Where it applies",
    json(withoutMessage(a.clauses)) !== json(withoutMessage(b.clauses)) && "What it checks",
  ].filter(Boolean);
}

/** What an edited copy was edited from: the original as a link, where it lives, and that it is off here. */
const editedFrom = original => h("span", null, ruleLink(original, titleOf(original)), `, in ${whereOf(original)}`, original.enabled ? "" : ". The original is off here.");

/** An edited copy's rows: when it was last edited, what it was edited from, and what changed. */
function editedCopyRows(stored, fmtDay) {
  const original = originalOf(stored), lastEdited = ["Last edited", fmtDay(lastEditedAt(stored) ?? stored.addedAt)];
  // The original is not known here (it was removed with its pack): its id says what this was edited from.
  if (!original) return [lastEdited, ["Edited from", stored.copiedFromRuleId]];
  const parts = changedParts(original, stored);
  return [lastEdited, ["Edited from", editedFrom(original)], ...(parts.length ? [["What changed", andList(parts)]] : [])];
}

/** On the original: that its edited copy runs in its place (or beside it, once switched back on), with a link. */
function originalRows(stored) {
  const copy = editedCopyOf(stored);
  if (copy === null) return [];
  const when = whenLabel(copy.addedAt);
  return [["Edited", h("span", null, `${when[0].toUpperCase()}${when.slice(1)}. `,
    stored.enabled ? "Your edited copy runs too, from Created by you. " : "Your edited copy runs in its place, from Created by you. ",
    ruleLink(copy, "See the edited copy"))]];
}

/** The rows of a rule's Source that say when it was added and edited; `fmtDay` is the sheet's own date format. */
export function editRows(stored, fmtDay) {
  if (isEditedCopy(stored)) return editedCopyRows(stored, fmtDay);
  const edited = lastEditedAt(stored);
  return [
    ["Added", fmtDay(stored.addedAt)],
    ...(edited === null ? [] : [["Last edited", fmtDay(edited)]]),
    ...originalRows(stored),
  ];
}
export function editRowsKey(stored) {
  const original = isEditedCopy(stored) ? originalOf(stored) : undefined, copy = original ? null : editedCopyOf(stored);
  return json([stored.addedAt, stored.updatedAt, stored.enabled, copy?.rule.id ?? null, copy?.addedAt ?? null,
    original?.enabled ?? null, original ? changedParts(original, stored) : null]);
}

/** What the toast says once rule `stored` is deleted from `repo`: for an edited copy, that its original is still off. */
export function deletedText(stored, repo) {
  const original = isEditedCopy(stored) ? originalOf(stored) : undefined;
  if (!original || original.enabled) return `Deleted from ${repo}.`;
  return `Deleted from ${repo}. “${titleOf(original)}” is still off in ${whereOf(original)}: turn it on there to go back.`;
}
