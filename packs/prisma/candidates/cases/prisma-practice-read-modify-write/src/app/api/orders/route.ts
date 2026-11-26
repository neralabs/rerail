import { NextResponse } from "next/server";
import { db } from "@/lib/db";

export async function POST(req: Request) {
  const { productId, qty } = await req.json();
  const product = await db.product.findUniqueOrThrow({ where: { id: productId } });
  if (product.stock < qty) {
    return NextResponse.json({ error: "sold out" }, { status: 409 });
  }
  await db.product.update({
    where: { id: productId },
    data: {
      stock: product.stock - qty, // caught
      sold: { increment: qty },
    },
  });
  return NextResponse.json({ ok: true });
}

export async function PUT(req: Request) {
  const { productId, qty } = await req.json();
  const product = await db.product.findUnique({ where: { id: productId } });
  if (!product) return NextResponse.json({ error: "not found" }, { status: 404 });
  const remaining = product.stock - qty;
  await db.product.update({ where: { id: product.id }, data: { stock: remaining } }); // caught
  return NextResponse.json({ remaining });
}

// Atomic decrement with a range guard: the database does the arithmetic.
export async function PATCH(req: Request) {
  const { productId, qty } = await req.json();
  const product = await db.product.findUnique({ where: { id: productId } });
  if (!product) return NextResponse.json({ error: "not found" }, { status: 404 });
  const res = await db.product.updateMany({
    where: { id: productId, stock: { gte: qty } },
    data: { stock: { decrement: qty } },
  });
  return NextResponse.json({ ok: res.count === 1 });
}

// Optimistic concurrency: the write only lands if nobody changed the row since the read.
export async function DELETE(req: Request) {
  const { productId, qty } = await req.json();
  const product = await db.product.findUniqueOrThrow({ where: { id: productId } });
  const res = await db.product.updateMany({
    where: { id: productId, version: product.version },
    data: { stock: product.stock - qty, version: { increment: 1 } },
  });
  if (res.count === 0) return NextResponse.json({ error: "retry" }, { status: 409 });
  return NextResponse.json({ ok: true });
}

// Compare-and-set on the field itself.
export async function HEAD(req: Request) {
  const { productId } = await req.json();
  const product = await db.product.findUniqueOrThrow({ where: { id: productId } });
  await db.product.update({
    where: { id: productId, sold: product.sold },
    data: { sold: product.sold + 1 },
  });
  return new Response(null);
}

// Not a count: string concatenation, and a field computed from another field.
export async function OPTIONS(req: Request) {
  const { productId, qty } = await req.json();
  const product = await db.product.findUniqueOrThrow({ where: { id: productId } });
  await db.product.update({
    where: { id: productId },
    data: { title: product.title + " (copy)", price: product.sold * qty },
  });
  return new Response(null);
}
