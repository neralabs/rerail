import { h, setAttr, setText, setClass } from "../dom.js";
import { fmt, plural } from "../format.js";
import { RANGES, ui, rangeLabel, data } from "../state.js";

let built = null;

function tile(now) {
  const el = h("div.tile", null, h("div.k"), h("div.v.num", null, h("span.big"), h("small")), h("div.s"));
  if (now) el.classList.add("now");
  return el;
}

function build(root) {
  const range = h("div.rangectl", { role: "group", "aria-label": "Time range" },
    RANGES.map(([d, l]) => h("button", { "data-act": "range", "data-i": d, "aria-pressed": "false" }, l)));
  const tiles = [tile(false), tile(false), tile(false), tile(true)];
  root.replaceChildren(
    h("div.rollhead", null, h("h2", null, "Overview"), h("p", null, "The first three follow the range. Rules on is current state."), range),
    h("div.tiles", null, tiles),
  );
  built = { range, tiles };
}

function setTile(el, k, big, small, s) {
  setText(el.querySelector(".k"), k);
  setText(el.querySelector(".big"), big);
  setText(el.querySelector("small"), small);
  setText(el.querySelector(".s"), s);
}

function rerailsNote(sum, label, empty) {
  if (sum.rerails) {
    const flagged = sum.flagged ? ` · ${fmt(sum.flagged)} more flagged only` : "";
    return `${label} · across ${sum.steerRules} ${plural(sum.steerRules, "rule")}${flagged}`;
  }
  if (empty) return "nothing to steer";
  const triggers = sum.triggers ? ` · ${fmt(sum.triggers)} ${plural(sum.triggers, "trigger")} flagged` : "";
  return `none ${label}${triggers}`;
}

function sessionsNote(sum, label) {
  const sir = sum.sessionsInRange;
  if (sir) return `${label} · ${fmt(Math.max(0, sir - sum.sessionsWithTriggers))} had no rules triggered`;
  return data.sessions.length ? `no sessions ${label}` : "your next agent session shows here";
}

export function renderOverview(root, sum) {
  if (built === null || !root.contains(built.range)) build(root);
  for (const b of built.range.children) setAttr(b, "aria-pressed", String(Number(b.dataset.i) === ui.range));
  const [t1, t2, t3, t4] = built.tiles;
  const label = rangeLabel();
  const empty = data.loaded && sum.rules.length === 0;
  setTile(t1, "Re-rails", fmt(sum.rerails), "", rerailsNote(sum, label, empty));
  const sir = sum.sessionsInRange;
  setTile(t2, "Sessions with triggers", sir ? fmt(sum.sessionsWithTriggers) : "0", sir ? ` of ${fmt(sir)}` : "", sessionsNote(sum, label));
  setTile(t3, "Rules triggered", fmt(sum.firedN), ` of ${fmt(sum.on.length)} on`,
    sum.firedN ? `${label} · ${fmt(sum.on.length - sum.firedOn)} on and quiet` : `none ${label}`);
  setTile(t4, "Rules on", fmt(sum.on.length), ` of ${fmt(sum.rules.length)}`,
    `${sum.presetsOff} By Rerail off · ${fmt(sum.neverTriggered)} never triggered`);
  setClass(root, "is-empty", empty);
}
