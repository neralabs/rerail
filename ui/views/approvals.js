/**
 * Waiting for approval (`#approvals`): every rule whose content waits for a person's approval, machine-wide (rules
 * created by you that a coding agent wrote or edited, and rules of packs added from a folder or tarball). Each shows
 * its whole content, whether the content approved before runs meanwhile or the rule runs nowhere yet, and, against
 * approved content, which lines change. Approve approves exactly the content shown (its `contentHash`); a rule that
 * changed since is refused, and the page reads it again.
 */
import { send } from "../api.js";
import { h, setContent, icon } from "../dom.js";
import { stamp } from "../format.js";
import { data, ui, packById } from "../state.js";
import { refreshNow } from "../poll.js";
import { invalidate, toast } from "../shell.js";
import { approvalView, awaitingCount, awaitingText, isRuleChanged } from "../approval-model.js";

export const needs = [];

const packName = id => packById(id)?.name ?? id;

const MARK = { same: "  ", added: "+ ", removed: "- " };

function contentBlock(view) {
  return h("pre.rdiff", { "aria-label": `Content of ${view.id}` },
    view.lines.map(line => h(`span.dl.dl-${line.op}`, null, `${MARK[line.op]}${line.text}`)));
}

function runsText(view) {
  if (view.runs === "nowhere") return "New: it runs nowhere until you approve it.";
  return `Edited: the content you approved ${stamp(view.approvedAt)} runs until you approve this one. Lines marked + are new, lines marked - go.`;
}

function entry(view) {
  const approving = ui.approving === view.key;
  return h("section.psec.appr", { "data-approval": view.key },
    h("h3", null, view.title),
    h("p.note2.muted", null, `${view.owner} · ${view.kind === "conversation" ? "conversation rule" : "code rule"} · `, h("span.mono", null, view.id)),
    h("p.pintro", null, runsText(view)),
    view.copiedFrom === null ? null
      : h("p.pintro", null, "An edited copy of ", h("span.mono", null, view.copiedFrom),
        ": approving it the first time switches the original off where this copy is added."),
    contentBlock(view),
    h("p.acts", null, h("button.btn.sm.solid", { "data-act": "approve", "data-i": view.key, disabled: approving },
      icon("check"), approving ? "Approving…" : "Approve")));
}

export function render(root) {
  const approvals = data.approvals;
  const views = (approvals ?? []).map(pending => approvalView(pending, packName));
  const signature = JSON.stringify([approvals === null, ui.approving,
    (approvals ?? []).map(p => [p.owner, p.id, p.contentHash, p.revision, p.approved?.approvedAt ?? null, p.packId && packName(p.packId)])]);
  const n = awaitingCount(approvals);
  setContent(root, signature, () => h("div.cpage.pbody.apage", null,
    h("h1", null, "Waiting for approval"),
    h("p.pintro", null, "A rule a coding agent writes or edits runs only with content you approved. Read each rule below, "
      + "then Approve runs it with exactly this content from the coding agent’s next turn."),
    approvals === null ? h("p.pintro.muted", null, "Loading…")
      : n === 0 ? h("p.pintro", null, "Nothing waits for approval.")
        : [h("p.pintro", null, `${awaitingText(n)}.`), views.map(entry)]));
}

async function approve(key) {
  const pending = (data.approvals ?? []).find(p => `${p.owner}:${p.id}` === key);
  if (pending === undefined || ui.approving !== null) return;
  const view = approvalView(pending, packName);
  ui.approving = key;
  invalidate();
  try {
    await send("POST", "/api/rule-approvals", view.body);
    toast(view.runs === "nowhere"
      ? `Approved. “${view.title}” runs where it is added from the coding agent’s next turn.`
      : `Approved. “${view.title}” runs with this content from the coding agent’s next turn.`);
  } catch (error) {
    toast(isRuleChanged(error)
      ? `Nothing approved: “${view.title}” changed since you read it. Read it again below before you approve it.`
      : `Could not approve ${view.id}: ${error.message}`, { error: true });
  } finally {
    ui.approving = null;
  }
  await refreshNow();
}

export const actions = { approve };
