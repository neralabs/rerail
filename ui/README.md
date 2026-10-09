# Dashboard

The browser view the daemon serves (`rerail ui`, `rerail daemon start --open`). It is plain JavaScript and CSS with no
build step and no framework. `npm run build:ui` (`scripts/build/copy-ui.mjs`) copies it into `dist/`, and the daemon
serves it with client-side routes falling back to `index.html`. It reads only the daemon's `/api` operations
(`api.js`).

- `app.js` and `state.js` hold the app and its state, `poll.js` refreshes it, `attachments.js` writes its rule pack
  changes in click order, and `render/` holds the views.
- `tokens.generated.css` holds the design tokens, generated from Nera's tokens by `scripts/generate/ui/tokens.mjs`;
  `npm run check:ui-tokens` checks them.

This is the shipped product UI. `playground/ui/` is a separate React tool for developing rules: it is never published
and does not share code with this one.
