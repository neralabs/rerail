import { ImageResponse } from "next/og";
import { prisma } from "@/lib/db"; // cross-lane: missed until the engine joins .ts and .tsx

export const runtime = "edge";

export async function GET(req: Request) {
  const id = new URL(req.url).searchParams.get("id") ?? "";
  const product = await prisma.product.findUnique({ where: { id } });
  return new ImageResponse(<div style={{ fontSize: 64 }}>{product?.name}</div>, { width: 1200, height: 630 });
}
