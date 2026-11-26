import { json } from "@remix-run/node";
import type { LoaderFunctionArgs } from "@remix-run/node";
import { prisma } from "~/db.server";

export async function loader({ params }: LoaderFunctionArgs) {
  const accounts = await prisma.account.findMany({ where: { userId: params.userId } }); // caught
  return json({ accounts });
}
