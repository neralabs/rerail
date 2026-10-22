# Dashboard

The browser view the daemon serves (`rerail ui`, `rerail daemon start --open`). It is plain JavaScript and CSS with no
build step and no framework. `npm run build:ui` (`scripts/build/copy-ui.mjs`) copies it into `dist/`, and the daemon
serves it with client-side routes falling back to `index.html`. It reads only the daemon's `/api` operations
(`api.js`).

- `index.html` is the frame: the top bar over both columns, the sidebar (`#sidenav`), the content column (`#content`)
  and the sheet (`#panel`). `app.js` renders it and routes clicks; `frame/` draws the top bar and the sidebar.
- `routes.js` holds the routes and the links from before v8 (`#rules`, `#activity`, `#rule-packs`), as pure
  functions; `shell.js` applies them and holds what every view shares (render on demand, toast, writes, the sheet,
  navigation); `actions.js` the actions any region may carry; `sheet.js` the sheet's.
- `views/` holds one module per view (`overview`, `pack`, `packs`, `create`, `history`, `sessions`, `settings`) and
  the pages shown in their place (`no-repo`, `missing`, `not-set-up`); `views/index.js` lists them and says what
  each module exports. `render/` holds what several of them share: the rule list and its controls, the range
  control, the sheet's content, the session timeline.
- `state.js` holds the state, `poll.js` refreshes it with what the page needs (`setNeeds`), `attachments.js` writes its
  rule pack changes in click order, `settings-writes.js` writes rule settings.
- `first-open.js` holds the Overview's pure derivations: the repository's first-open state (which banner, which empty
  block), the Violations steered tally and the Most violations logged ranking.
- `pack-model.js` holds the pack pages', the catalog's and the pack sheet's pure derivations: a page's rules as
  filtered and sorted, the whole-pack switch's state (mixed included), the catalog's kinds and order.
- `css/base.css` styles the frame and the shared pieces, `css/sheet.css` the sheet, and `css/<view>.css` each view.
  `tokens.generated.css` holds the design tokens, generated from Nera's tokens by `scripts/generate/ui/tokens.mjs`;
  `npm run check:ui-tokens` checks them, and that `index.html` links every sheet under `css/`.

This is the shipped product UI. `playground/ui/` is a separate React tool for developing rules: it is never published
and does not share code with this one.
