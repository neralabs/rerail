import { h, setAttr, setText, setClass } from "../dom.js";
import { fmt, plural } from "../format.js";
import { RANGES, ui, rangeLabel, data } from "../state.js";

let built = null;

const tile = () => h("div.tile", null, h("div.k"), h("div.v.num", null, h("span.big"), h("small")), h("div.s"));

/** The range control; Rules' overview and Activity each carry one, both set `ui.range`. */
export function rangeControl() {
  return h("div.rangectl", { role: "group", "aria-label": "Time range" },
    RANGES.map(([d, l]) => h("button", { "data-act": "range", "data-i": d, "aria-pressed": "false" }, l)));
}

export function syncRange(control) {
  for (const b of control.children) setAttr(b, "aria-pressed", String(Number(b.dataset.i) === ui.range));
}

function build(root) {
  const range = rangeControl();
  const tiles = [tile(), tile(), tile(), tile()];
  root.replaceChildren(h("div.rollhead", null, h("h2", null, "Overview"), range), h("div.tiles", null, tiles));
  built = { range, tiles };
}

function setTile(el, k, big, small, s) {
  setText(el.querySelector(".k"), k);
  setText(el.querySelector(".big"), big);
  setText(el.querySelector("small"), small);
  setText(el.querySelector(".s"), s);
}

function steeredNote(sum, label) {
  if (sum.rerails === 0) return sum.rules.length === 0 ? "nothing to steer yet" : `none ${label}`;
  const logged = sum.logged ? ` · ${fmt(sum.logged)} logged only` : "";
  return `${label} · across ${sum.steerRules} ${plural(sum.steerRules, "rule")}${logged}`;
}

function sessionsNote(sum, label) {
  const sir = sum.sessionsInRange;
  if (sir) return `${label} · ${fmt(Math.max(0, sir - sum.sessionsWithTriggers))} had none`;
  return sum.anySessions ? `no sessions ${label}` : "your next coding agent session shows here";
}

export function renderOverview(root, sum) {
  if (built === null || !root.contains(built.range)) build(root);
  syncRange(built.range);
  const [t1, t2, t3, t4] = built.tiles;
  const label = rangeLabel();
  setTile(t1, "Steered", fmt(sum.rerails), "", steeredNote(sum, label));
  const sir = sum.sessionsInRange;
  setTile(t2, "Sessions with violations logged", sir ? fmt(sum.sessionsWithTriggers) : "0", sir ? ` of ${fmt(sir)}` : "", sessionsNote(sum, label));
  setTile(t3, "Rules with violations logged", fmt(sum.firedN), ` of ${fmt(sum.on.length)}`,
    sum.firedN ? `${label} · ${fmt(sum.on.length - sum.firedOn)} on with none` : `none ${label}`);
  setTile(t4, "Rules on", fmt(sum.on.length), ` of ${fmt(sum.rules.length)}`,
    `${fmt(sum.logOn)} log · ${fmt(sum.steerOn)} steer · 0 block`);
  setClass(root, "is-empty", data.loaded && sum.rules.length === 0);
}
