import { NextRequest, NextResponse } from "next/server";
import { prisma } from "@/lib/db";

export async function GET(req: NextRequest) {
  const q = req.nextUrl.searchParams.get("q") ?? "";
  const sort = req.nextUrl.searchParams.get("sort") ?? "name";
  const products = await prisma.$queryRawUnsafe(
    `SELECT id, name FROM "Product" WHERE name ILIKE '%${q}%' ORDER BY "${sort}"`, // caught
  );
  return NextResponse.json(products);
}

export async function DELETE(req: NextRequest) {
  const id = req.nextUrl.searchParams.get("id");
  await prisma.$executeRawUnsafe('DELETE FROM "Cart" WHERE id = ' + id); // caught
  await prisma.$executeRawUnsafe("DELETE FROM \"CartItem\" WHERE \"cartId\" = '" + id + "'"); // caught
  return new NextResponse(null, { status: 204 });
}

export async function POST(req: NextRequest) {
  const { email, name } = await req.json();
  // Parameters: the values never enter the SQL text.
  await prisma.$executeRawUnsafe('UPDATE "User" SET name = $1 WHERE email = $2', name, email);
  const users = await prisma.$queryRaw`SELECT id FROM "User" WHERE email = ${email}`;
  return NextResponse.json(users);
}
