// Near miss: this package does not use TanStack Query, so a state it calls dehydratedState (here Apollo's cache
// extract) is not the query cache this rule is about.
export function page(body: string, dehydratedState: unknown) {
  return `<!doctype html><div id="root">${body}</div><script>window.__APOLLO__ = ${JSON.stringify(dehydratedState)}</script>`;
}
