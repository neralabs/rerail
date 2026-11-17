/**
 * Merge Assist's Settings fold, drawn into the page `merge.js` builds (`page.settings`): its summary says the saved
 * values (the conflict hook command and its timeout), and its form holds the person's edits (`ui.mergeSettings`)
 * until Save, which `merge.js` sends. The form is rebuilt only when the saved values or the checkout change; why the
 * last Save was refused and Save's busy state are patched in place.
 */
import { h, setAttr, setContent, setText } from "../../dom.js";
import { ui } from "../../data/state.js";
import { HOOK_TIMEOUT_DEFAULT, HOOK_TIMEOUT_MAX, settingsDraft, settingsSummary } from "../../model/merge-model.js";

/** The Settings form's fields: the person's edits while there are any, else the saved values (`status.settings`). */
export const settingsFields = status => ui.mergeSettings ?? { ...settingsDraft(status.settings), problem: null, field: null };

/** The Settings fold: its summary (the saved values) and the form, built per saved settings (`renderSettings`). */
export function buildSettings() {
  const summary = h("summary", { "data-act": "mafold", "data-i": "settings" }), form = h("div.msetform");
  return { root: h("details.mfold.msettings", { "aria-label": "Merge Assist settings" }, summary, h("div.in", null, form)), summary, form };
}

/**
 * A text field of the Settings form, holding `value` when built; typing keeps it in `ui.mergeSettings` (`onInput` in
 * `merge.js`).
 */
function field(spec, attrs, value) {
  const input = h(spec, { autocomplete: "off", spellcheck: "false", ...attrs });
  input.value = value;
  return input;
}

/** The form, built with `fields` (the person's edits, else the saved values); `patchSettings` keeps the rest current. */
function settingsForm(fields) {
  return [
    h("div.mfieldrow", null,
      h("label.k", { for: "ma-hook" }, "Conflict hook command"),
      field("input.mfield.mono", { id: "ma-hook", type: "text", placeholder: "[\"./scripts/on-conflict.sh\"]", "aria-describedby": "ma-hook-hint ma-settings-problem" }, fields.command),
      h("p.mnote2", { id: "ma-hook-hint" }, "A JSON array: the program, then its arguments. Rerail runs it without a shell from the checkout’s root when a conflict is detected or closes, with the conflict as JSON on stdin. Empty for none.")),
    h("div.mfieldrow", null,
      h("label.k", { for: "ma-timeout" }, "Timeout in seconds"),
      // A text field: a number field reads "" for text it can't parse, which would save the default the person never typed.
      field("input.mfield.mnum", { id: "ma-timeout", type: "text", inputmode: "numeric", "aria-describedby": "ma-timeout-hint ma-settings-problem" }, fields.timeout),
      h("p.mnote2", { id: "ma-timeout-hint" }, `1 to ${HOOK_TIMEOUT_MAX}; ${HOOK_TIMEOUT_DEFAULT} by default. A command still running then is stopped.`)),
    h("div.macts", null,
      h("button.btn.sm.solid", { "data-act": "masettings" }, "Save"),
      // Always rendered and empty without a problem: a screen reader announces a live region's change only once it is shown.
      h("p.msetproblem", { id: "ma-settings-problem", "aria-live": "polite" })),
  ];
}

/**
 * Why the last Save was refused, and which field it was about (`aria-invalid`); empty once a field changes. The
 * paragraph is never hidden, so the text a refused Save puts in it is announced.
 */
export function patchProblem(form, fields) {
  setText(form.querySelector('p[id="ma-settings-problem"]'), fields.problem ?? "");
  setAttr(form.querySelector('input[id="ma-hook"]'), "aria-invalid", fields.field === "command" ? "true" : null);
  setAttr(form.querySelector('input[id="ma-timeout"]'), "aria-invalid", fields.field === "timeout" ? "true" : null);
}

/** While a save or a switch is out: the fields read-only, Save disabled and saying so. */
function patchSaving(form) {
  const busy = ui.mergeSaving || ui.mergeSwitching;
  for (const input of form.querySelectorAll("input.mfield")) setAttr(input, "readonly", busy);
  const save = form.querySelector('button[data-act="masettings"]');
  setAttr(save, "disabled", busy);
  setAttr(save, "aria-busy", ui.mergeSaving ? "true" : null);
  setText(save, ui.mergeSaving ? "Saving…" : "Save");
}

/** The problem the last Save found, and Save's busy state, patched in place. */
function patchSettings(form, fields) {
  patchProblem(form, fields);
  patchSaving(form);
}

/**
 * The Settings fold while on: the saved values in its summary, the form rebuilt only when they or the checkout change
 * (another checkout's form must not keep what was typed there).
 */
export function renderSettings(page, status) {
  const s = page.settings, show = status !== null && status.enabled;
  setAttr(s.root, "hidden", !show);
  if (!show) return;
  const fields = settingsFields(status);
  setAttr(s.root, "open", ui.mergeOpen.has("settings"));
  setText(s.summary, settingsSummary(status.settings));
  setContent(s.form, JSON.stringify([status.repositoryKey, status.checkout, status.settings]), () => settingsForm(fields));
  patchSettings(s.form, fields);
}
