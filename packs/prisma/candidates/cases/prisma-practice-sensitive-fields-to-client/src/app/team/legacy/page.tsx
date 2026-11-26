import { prisma } from "@/lib/db";
import { LegacyTable } from "@/components/legacy-table";

export default async function LegacyTeamPage() {
  // cross-lane: missed until the engine joins .ts and .tsx (this .tsx page and the .jsx client component)
  const rows = await prisma.user.findMany();
  return <LegacyTable rows={rows} />;
}
