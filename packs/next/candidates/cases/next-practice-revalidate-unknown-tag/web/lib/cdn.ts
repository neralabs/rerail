// Responses Vercel's CDN caches under the tags of their Vercel-Cache-Tag header; revalidateTag purges those.
export function pageTags(domain: string) {
  return `page:${domain},page`;
}

export function renderPage(domain: string, body: string) {
  return new Response(body, { headers: { "Vercel-Cache-Tag": pageTags(domain) } });
}

export function wellKnown(domain: string) {
  const res = new Response("{}");
  res.headers.set("Vercel-Cache-Tag", `wellknown:${domain}`);
  return res;
}
