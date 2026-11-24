import * as RQ from "@tanstack/react-query";

// Vite SSR: the template's placeholder is replaced with the state script.
export function injectState(html: string, qc: RQ.QueryClient) {
  return html.replace("<!--app-state-->", `<script>window.__RQ__=${JSON.stringify(RQ.dehydrate(qc))}</script>`); // caught
}

export function injectBareState(html: string, qc: RQ.QueryClient) {
  // gap: the markup sits in index.html, not around the call, so nothing here says this is HTML.
  return html.replace("__RQ_STATE__", JSON.stringify(RQ.dehydrate(qc)));
}

// The state wrapped in an object is written out with it.
export function injectWrapped(html: string, qc: RQ.QueryClient, user: { name: string }) {
  const dehydratedState = RQ.dehydrate(qc);
  const wrapped = `<script type="application/json">${JSON.stringify({ queries: RQ.dehydrate(qc), user })}</script>`; // caught
  const short = `<script>window.__APP__=${JSON.stringify({ dehydratedState, user })}</script>`; // caught
  // Near misses: a JSON response with the state in an object, and an object without the state.
  const body = JSON.stringify({ dehydratedState });
  const userOnly = `<script>window.__USER__=${JSON.stringify({ user }).replace(/</g, "\\u003c")}</script>`;
  return { html: html.replace("<!--app-state-->", wrapped + short + userOnly), body };
}
