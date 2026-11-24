import { getPosts } from "@/lib/data";

export async function GET() {
  return Response.json(await getPosts());
}
