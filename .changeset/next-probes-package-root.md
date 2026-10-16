---
"@neralabs/rerail-pack-next": patch
---

Next probes: the app-router, pages-router, route-handler, segment-config, middleware and client-hook probes read `app/`, `src/app/`, `pages/` and `middleware` paths relative to the nearest `package.json` that depends on `next`, so a Next app in a subfolder (`apps/web/app/page.tsx`) is checked and an `app/` folder of a package without `next` is not.
