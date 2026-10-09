# integration

End-to-end tests against the built package (`dist/`) and a real standalone OMP host: `npm run test:integration`
builds and runs them, `npm run test:integration:built` reuses an existing build. The OMP tests need
`RERAIL_OMP_EXECUTABLE`, the path `node scripts/host/install-omp-host.mjs` installs and prints.

They import `src/` only through a layer's `index.ts` (`npm run check:boundaries`). Shared helpers, such as the OMP
test host and the cross-host parity harness, live in `support/`.
