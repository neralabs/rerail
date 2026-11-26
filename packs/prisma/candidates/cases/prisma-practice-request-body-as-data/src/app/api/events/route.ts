import { db } from "../../../lib/db";
import { getSession } from "../../../lib/auth";

export async function POST(req: Request) {
  const session = await getSession();
  const { id, ...rest } = await req.json();
  await db.post.update({ where: { id }, data: rest }); // caught
  const body = await req.json();
  await db.post.create({ data: { ...body, authorId: session.user.id } }); // caught
  // The whole body is one Json column's value, not the row.
  await db.webhookEvent.create({ data: { payload: body } });
  return new Response(null, { status: 204 });
}
