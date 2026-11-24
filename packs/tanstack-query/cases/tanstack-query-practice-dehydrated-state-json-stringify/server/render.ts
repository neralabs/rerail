import type { Request, Response } from "express";
import { QueryClient, dehydrate } from "@tanstack/react-query";
import { renderToString } from "react-dom/server";
import serialize from "serialize-javascript";
import superjson from "superjson";
import { App } from "../src/App";
import { productQuery } from "../src/queries";

export async function renderPage(req: Request, res: Response) {
  const queryClient = new QueryClient();
  await queryClient.prefetchQuery(productQuery(req.params.id));
  const dehydratedState = dehydrate(queryClient);
  const body = renderToString(App({ queryClient }));

  const state = `<script>window.__REACT_QUERY_STATE__ = ${JSON.stringify(dehydratedState)};</script>`; // caught
  res.send(`<!doctype html><html><body><div id="root">${body}</div>${state}</body></html>`);
}

export async function renderLegacy(req: Request, res: Response) {
  const qc = new QueryClient();
  await qc.prefetchQuery(productQuery(req.params.id));
  const state = dehydrate(qc);
  const head = "<script>window.__RQ__ = " + superjson.stringify(state) + "</script>"; // caught
  res.send(head + renderToString(App({ queryClient: qc })));
}

export async function renderEscaped(req: Request, res: Response) {
  const qc = new QueryClient();
  await qc.prefetchQuery(productQuery(req.params.id));
  const state = dehydrate(qc);
  // Escaped: `<` cannot close the script element any more.
  res.send(`<script>window.__RQ__ = ${JSON.stringify(state).replace(/</g, "\\u003c")}</script>`);
  // serialize-javascript escapes HTML characters.
  res.write(`<script>window.__RQ2__ = ${serialize(state, { isJSON: true })}</script>`);
}

export async function stateAsJson(req: Request, res: Response) {
  const qc = new QueryClient();
  await qc.prefetchQuery(productQuery(req.params.id));
  // A JSON response, not HTML: the browser never parses it as markup.
  res.type("json").send(JSON.stringify(dehydrate(qc)));
  // A deep copy that drops undefined values, as some apps do before passing the state on.
  return JSON.parse(JSON.stringify(dehydrate(qc)));
}
