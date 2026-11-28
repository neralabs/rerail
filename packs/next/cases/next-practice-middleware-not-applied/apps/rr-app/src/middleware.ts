// A React Router app's own request middleware module: Next.js never loads it, and it is no Next.js file.
export function middleware(request: Request) {
  return request;
}

export const config = { matcher: ["/dashboard/:path*"] };
