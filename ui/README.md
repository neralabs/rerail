# Dashboard

The browser view the daemon serves (`rerail manage`, `/rerail-manage`, `rerail daemon start --open`). It is plain
JavaScript and CSS with no build step and no framework. `npm run build:ui` (`scripts/build/copy-ui.mjs`) copies it
into `dist/`, and the daemon serves it with client-side routes falling back to `index.html`. It reads only the
daemon's `/api` operations and `/healthz` (`api.js`).

- `index.html` is the frame: the top bar over both columns, the sidebar (`#sidenav`), the content column (`#content`)
  and the sheet (`#panel`). `app.js` renders it and routes clicks to the region's actions.
- `frame/` draws the frame: `topbar.js` the top bar (sidebar toggle, the wordmark, which opens Overview, repository
  pill and menu, status) and `sidenav.js` the sidebar (Overview, one item per added rule pack with how many of its
  rules are on, Created by you, Add rule packs, Create a rule, History by rule and by session, Merge Assist with how
  many of its conflicts are open, Settings).
- `routes.js` holds the routes as pure functions of the URL: `#overview`, `#pack/<id>`, `#created`, `#packs`,
  `#packs?review=1`, `#create`, `#history` (`?rule=<id>`, and `?outcome=steered|logged` for its chip), `#sessions`
  (`?filter=violations|in-progress` for its chip), `#settings`, `#merge` (`?conflict=<id>` brings that conflict into
  view, as the session's line links it) and `#rule/<id>`, and the links from before
  v8 (`#rules`, `#activity`, `#rule-packs`). `shell.js` applies them and holds what every view shares: render on
  demand, the toast and clipboard, the theme, the writes, opening and closing the sheet, navigation.
- `actions.js` holds the actions any region may carry (open the sheet, move to a view or repository, a rule's
  switch, a pack's Add and Remove, the time range); `sheet.js` the sheet's own and what a poll loads while it is open.
- `views/` holds one module per view (`overview`, `pack` for a pack page and Created by you, `packs` for the catalog
  and the review, `create`, `history`, `sessions`, `settings`, `merge` for Merge Assist) and the pages shown in their
  place (`no-repo`, `missing`, `not-set-up`); `views/index.js` lists them and says what each module exports.
  Merge Assist's page shows, while it is on, its status strip, its Settings (where conflicts reach the person, and
  the conflict hook command) and the conflicts with work in flight: open ones as cards, loudest first, with Dismiss
  (not a conflict) as the one action, then the dismissed ones folded with who dismissed each and when. It renders
  other people's titles, summaries, hunks, names and links as text nodes only, and links a pull request only when
  `safePeerUrl` accepts it.
- `render/` holds what several views or the sheet share: a pack page's rule table and its controls (`rules.js`,
  `rule-controls.js`), the range control, the repository choices (`repos.js`), the rule sheet (`panel.js`, its
  state and clicks in `rule-sheet-actions.js`, its pure text and prompts in `rule-sheet-model.js`), a rule pack in the
  sheet (`pack-panel.js`), and the session timeline (`timeline.js`).
- `state.js` holds the state; `poll.js` refreshes it with what the page needs (`setNeeds`);
  `repo-rule-counts.js` reads each repository's rule counts; `attachments.js` writes
  rule pack changes in click order, `rule-remove.js` deletes a rule created by you from the repository (with Undo),
  and `settings-writes.js` writes rule settings. Merge Assist's writes go through `writeMergeAssist` in `poll.js`:
  each answers the checkout's status, which replaces the page's unless a later write was sent, and a read sent before
  the write answered is dropped.
- Pure derivations, without the DOM: `derive.js` (a rule's title, mode, scope and a pack's suppression comments),
  `first-open.js` (the Overview: the repository's first-open state, the Violations steered tally, the Most violations
  logged ranking, whether a session is live), `pack-model.js` (a pack page's rules as filtered and sorted, the
  whole-pack switch's state, the catalog's kinds and order), `history-model.js` (which rules History by rule lists
  under its All · Steered · Logged chips and which one it shows), `sessions-model.js` (a session's events as turns
  and tool calls), `settings-model.js` (coding agent states and the install date), `repo-rules.js` (what Settings and
  the repository menu say of a repository's rule counts) and `merge-model.js` (Merge Assist's conflicts and
  their order, its status strip, readiness checklist and Settings form, the sidebar's count, and which pull request
  links are safe to open).
  `format.js` formats numbers and dates; `dom.js` builds and reconciles elements.
- `css/base.css` styles the frame and the shared pieces, `css/sheet.css` the sheet, and `css/<view>.css` each view.
  `tokens.generated.css` holds the design tokens, generated from Nera's tokens by `scripts/generate/ui/tokens.mjs`;
  `npm run check:ui-tokens` checks them, and that `index.html` links every sheet under `css/`. The dashboard's own
  tokens (titles, outcomes, the inset surface, the page background, shadows) are in one block at the top of
  `base.css`, each a Nera token or a mix of them; no colour literal appears elsewhere in the CSS.

The browser specs in `e2e/` drive it over seeded scenarios (`e2e/README.md`); `test/ui/` unit-tests its pure modules,
and renders the Merge Assist page over a small fake DOM (`test/ui/support/fake-dom.ts`).
