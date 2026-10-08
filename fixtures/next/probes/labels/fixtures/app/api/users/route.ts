export const runtime = "deno";

export async function GET() {
  return Response.json([]);
}

export const POST = async (request: Request) => new Response(await request.text(), { status: 201 });

function helper() {}
export { helper };
