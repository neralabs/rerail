import bcrypt from "bcrypt"; // caught

// The app's package.json has no "name": the route is still an edge entry of a Next.js 15 app.
export const runtime = "edge";

export async function GET() {
  return new Response(await bcrypt.hash("avatar", 10));
}
