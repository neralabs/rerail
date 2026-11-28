import { NextResponse } from "next/server";

// Forwards a request to the internal API; it exports no `proxy` and no default.
export async function proxyToApi(request: Request, path: string) {
  const upstream = await fetch(`${process.env.API_URL}${path}`, {
    method: request.method,
    headers: request.headers,
    body: request.method === "GET" ? undefined : await request.text(),
  });
  return new NextResponse(upstream.body, { status: upstream.status });
}
