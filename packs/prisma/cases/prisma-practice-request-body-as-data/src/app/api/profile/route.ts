import { NextRequest, NextResponse } from "next/server";
import { db } from "@/lib/db";
import { getSession } from "@/lib/auth";
import { UpdateProfile } from "@/lib/schemas";

export async function PATCH(req: NextRequest) {
  const session = await getSession();
  const body = await req.json();
  const user = await db.user.update({ where: { id: session.user.id }, data: body }); // caught
  return NextResponse.json(user);
}

export async function PUT(request: Request) {
  const session = await getSession();
  const user = await db.user.update({
    where: { id: session.user.id },
    data: (await request.json()) as { name: string }, // caught
  });
  return NextResponse.json(user);
}

export async function POST(req: NextRequest) {
  const session = await getSession();
  const body = await req.json();
  // Parsed: zod drops the keys the schema does not list.
  const input = UpdateProfile.parse(body);
  await db.user.update({ where: { id: session.user.id }, data: input });
  // A safeParse result's data.
  const parsed = UpdateProfile.safeParse(body);
  if (!parsed.success) return NextResponse.json(parsed.error, { status: 400 });
  await db.user.update({ where: { id: session.user.id }, data: { ...parsed.data } });
  // Fields picked one by one.
  await db.user.update({ where: { id: session.user.id }, data: { name: body.name, bio: body.bio } });
  return NextResponse.json({ ok: true });
}

export async function DELETE(req: NextRequest) {
  const session = await getSession();
  let body = await req.json();
  body = UpdateProfile.parse(body);
  await db.user.update({ where: { id: session.user.id }, data: body });
  return NextResponse.json({ ok: true });
}
