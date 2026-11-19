# Rerail feedback: turning a design-rules list into Rerail rules

A record of one Claude Code session in the `interlace` repo (Expo + NativeWind app), shared with
the Rerail team. The product owner pasted 18 design rules into `/rerail-create`. The session turned
8 of them into code rules and added and approved them. It then hit a dashboard limit: **the rule
title shown in the UI is the agent's message, not the person's wording.**

- Date: 2026-11-19
- Rerail: 0.1.5 (`~/.rerail/current`)
- Agent: Claude Code, Claude Opus 5.5
- Rerail found nothing during the session itself. Every finding below came from drafting, testing
  and using the dashboard.

## The main ask: a title in the person's words

The product owner's words after seeing the dashboard (screenshot: `dashboard-rule-titles.jpg`):

> for rerail, use my wording, what I pasted as the rule title, not what agent sees, I'm expecting
> to see the rules in UI as I pasted

What we found:

- The dashboard builds a user rule's title in `ruleTitle(stored)` (dist): it uses
  `rule.clauses[0].violation.message`, falling back to `rule.id`. Only pack rules have
  `rule.title`.
- `rerail.policy/v1` has no title field. Adding `"title"` to a rule fails:
  `rules test` returns `rule: Unrecognized key: "title"` (`drafts/probe-title-field.json`).
- The result: each card in the rule list shows imperative advice written for the agent, like
  "Take this shadow (and its shadow color) off; use a 1 px `border border-border` if it needs an
  edge…". The rule the person wrote ("No shadows except the one sheet shadow") appears nowhere,
  and the `description` only shows in the side panel.
- The only workaround is to prepend the person's sentence to the first clause's message. That
  changes what the agent sees and uses up the 200-character message budget.

**Request:** an optional `title` on user rules (`rerail.policy/v1`), shown in the dashboard list
and panel header, falling back to today's behaviour. `/rerail-create` would fill it with the
person's own sentence, and the read-back card would show it.

The words the product owner would expect as titles (from the pasted list):

| Rule id | Their wording |
|---|---|
| `local:color-tokens-only` | Colors come only from tokens in `src/theme/tokens.ts`. No hex, rgb(), hsl() or Tailwind palette colors in `src/`. |
| `local:errors-use-attention` | The only meaningful colors are neutral and `attention`… Errors use `attention`, not red. |
| `local:type-through-text-variants` | Text sizes come only from the type scale in DESIGN.md, through `<Text variant="...">`. Font weights come only from the scale. |
| `local:spacing-on-4pt-scale` | Spacing is on the 4-point scale only: 4, 8, 12, 16, 20, 24, 32, 40, 48. No half steps and no arbitrary values. |
| `local:radius-from-tokens` | Radius comes only from `radius.json` (card 14, sheet 22, button 12, chip 999). |
| `local:no-shadows` | No shadows except the one sheet shadow. |
| `local:lucide-icons-only` | No new UI library or icon set. Icons come only from `lucide-react-native`. |
| `local:icon-buttons-have-labels` | Every icon-only button has an `accessibilityLabel`. |

## What was pasted (verbatim)

```
## Design hard rules (break one = the task isn't done)

### Color
1. Colors come only from tokens in `src/theme/tokens.ts`. No hex, rgb(), hsl() or
   Tailwind palette colors (`red-500`, `gray-200`) in `src/`.
2. The only meaningful colors are neutral and `attention`. `success` appears only in short
   confirmations. No other color tokens (no `destructive`, no category colors) without my
   written approval in DESIGN.md first.
3. Errors use `attention`, not red.

### Type
4. Text sizes come only from the type scale in DESIGN.md, through `<Text variant="...">`.
   No `text-xs/sm/base/lg/xl/2xl/3xl` and no `text-[Npx]` in screens or features.
5. Font weights come only from the scale (400, 500, 600, 700, 800 Figtree). No
   `font-medium` etc. outside the Text variants.

### Spacing and shape
6. Spacing is on the 4-point scale only: 4, 8, 12, 16, 20, 24, 32, 40, 48. No half steps
   (`gap-0.5`, `gap-1.5`, `px-3.5`) and no arbitrary values (`m-[7px]`).
7. Screen side padding is 20 (`px-5`), set once in the screen wrapper, not per screen.
8. Radius comes only from `radius.json` (card 14, sheet 22, button 12, chip 999).
9. No shadows except the one sheet shadow.

### Components
10. Before creating a component, search `src/components/` and reuse or extend what exists.
    A new shared component needs a one-line reason in the PR.
11. Screens don't style raw `View`/`Text`/`Pressable` for things a component already covers
    (headers, sheets, cards, empty states, buttons).
12. No new UI library or icon set. Icons come only from `lucide-react-native`.

### When the doc is silent
13. If DESIGN.md doesn't say how something looks or behaves (error, loading, disabled,
    offline, motion, Android behavior), STOP and ask. Don't fall back to shadcn, Tailwind
    or iOS defaults. Then add the answer to DESIGN.md.

### Behavior the doc already requires
14. Every save success and primary action triggers a light haptic.
15. Every icon-only button has an `accessibilityLabel`. Every card reads as one element
    to screen readers.
16. Every pressable has a visible pressed state and a 44 pt minimum target.

### Done means
17. Each UI task ships with screenshots of: light, dark, largest text size, French, and
    the empty/error/long-content states. If I couldn't produce one, say which and why.
18. Run the design check script (`npm run lint`) and report zero design violations.
```

## What the session did

1. Read the skill, `code-rules.md` and `rule-quality.md`. Counted every pattern in `src/` to see
   how much the current code already breaks each rule.
2. Sorted the 18 rules into checkable and not checkable. It drafted 8 code rules from 1–6, 8, 9,
   12 and 15 (first half), with caught and passed cases, and generated the JSON with
   `drafts/generate-drafts.mjs`.
3. Ran `rules test --json` on each draft (`test-reports/`). Every case passed.
4. A fresh subagent reviewed the drafts against `rule-quality.md`. The session fixed what it
   found (list below) and tested again.
5. Showed one read-back card per rule. The product owner answered "all yes, radius only the
   four". The session then ran `rules add` + `rules approve` for each. The final state is in
   `exported-rules.json`.

| Rule | From | Mode | Repo matches at add |
|---|---|---|---|
| `local:color-tokens-only` | 1 | Steer | 1 (button.tsx `text-white`) |
| `local:errors-use-attention` | 2, 3 | Steer | 9 in 8 files |
| `local:type-through-text-variants` | 4, 5 | **Log** | 77 in 14 files |
| `local:spacing-on-4pt-scale` | 6 | Steer | 21 in 12 files |
| `local:radius-from-tokens` | 8 | Steer | 4 |
| `local:no-shadows` | 9 | Steer | 4 (button.tsx) |
| `local:lucide-icons-only` | 12 | Steer | 0 |
| `local:icon-buttons-have-labels` | 15 (part) | Steer | 0 |

The type rule is in Log because `text.tsx` still has stock shadcn variants that don't match the
DESIGN.md scale. Steering now would push the agent toward the wrong variants.

Not turned into rules, because they can't be checked mechanically or need code that doesn't exist
yet: 7 (no screen wrapper yet), 10, 11, 13, 14, 15 (second half), 16, 17, 18 (`npm run lint` is
plain `eslint-config-expo`, with no design checks).

### Reviewer findings that changed the drafts

- `shadow-black/5` matched the color rule and the shadow rule, with conflicting advice. Removed
  `shadow` from the color rule's utilities.
- The settings "Delete account" row uses `text-destructive`. Steering it to attention would give
  a delete action the "needs action" color, so the message now says delete actions stay neutral.
  Also excluded `src/components/ui/` (shadcn variant definitions) and added a
  `variant="destructive"` clause.
- "Round to the nearer step" was ambiguous for `gap-0.5`, so the message now lists the mapping.
  Negative 1 px offsets (`mt-[-1px]`) are allowed. Off-scale padding that clears a tab bar gets a
  "pad by its measured height" hint.
- The shadow message claimed the sheet shadow lives in `src/components/sheet.tsx`. It doesn't
  (the sheet is a native `formSheet`), so the claim and the exemption were removed.
- Allowed `borderRadius: 0`. Added `expo-symbols` to the icon list. Treated a JSX spread
  (`{...a11y}`) as a possible label.

## Other friction worth knowing

- **One scope per rule.** Rules 4 and 5 had slightly different scopes in the person's words
  ("in screens or features" vs. "outside the Text variants"). A clause can't have its own scope,
  so the session merged them into one rule with one scope.
- **Confusing grammar error on a `.tsx`-only rule.** A clause with `kind: jsx_spread_attribute`
  failed with an error that also said `jsx_opening_element is invalid (typescript grammar)`. With
  `language: "typescript"` the rule is checked against several grammars, and the error names
  kinds that are fine for `.tsx`, which is the only file type in scope. The fix was
  `kind: jsx_expression` with a `^\{\s*\.\.\.` regex. An error tied to the files the rule's scope
  can actually reach would have saved a round.
- **`rules add` prints the whole rule JSON.** With 8 rules in a row, the useful lines
  (`● Approved rule …`) got lost in hundreds of lines of output.
- **Log rules show the same "On" toggle as Steer rules** in the list. In the screenshot,
  `type-through-text-variants` is in Log but looks the same as the Steer rules beside it.
- **Agent-side mistake (not Rerail):** the session put the launcher in a zsh variable, and the
  call failed with `no such file or directory: …/node …/cli.js`. The skill warns about this.

## Files

- `dashboard-rule-titles.jpg`: the product owner's screenshot of the rule list and panel
- `drafts/*.json`: each draft as tested, with its cases
- `drafts/generate-drafts.mjs`: the script that generated the drafts
- `drafts/probe-title-field.json`: the `title` probe that was refused
- `test-reports/*.json`: the final `rules test --json` output for each draft
- `exported-rules.json`: `rules export` after adding (paths redacted)
