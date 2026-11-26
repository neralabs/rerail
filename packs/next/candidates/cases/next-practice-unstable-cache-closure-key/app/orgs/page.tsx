import { unstable_cache } from "next/cache";

import { getSession } from "@/lib/auth";
import { prisma } from "@/lib/db";

export default async function OrgsPage() {
  const session = await getSession();
  // keyParts hold the user's id, but the read is the active organization, another field of the same session: a
  // user who switches organization keeps getting the first one's row.
  const getOrg = unstable_cache( // caught
    async () => prisma.org.findFirst({ where: { id: session.user.orgId } }),
    ["org", session.user.id],
  );
  // Covered: keyParts hold the field the read uses, a value above it, or a value derived from it.
  const getOwnOrg = unstable_cache(
    async () => prisma.org.findFirst({ where: { id: session.user.orgId, ownerId: session.user.id } }),
    ["own-org", session.user.orgId, session.user.id],
  );
  const getMembership = unstable_cache(
    async () => prisma.member.findFirst({ where: { userId: session.user.id, orgId: session.user.orgId } }),
    ["membership", JSON.stringify(session.user)],
  );
  const getProfile = unstable_cache(
    async () => prisma.user.findFirst({ where: { id: session.user.id } }),
    ["profile", session.user.id.toString()],
  );
  const [org, own, membership, profile] = await Promise.all([getOrg(), getOwnOrg(), getMembership(), getProfile()]);
  return <p>{[org?.name, own?.name, membership?.role, profile?.name].join(" / ")}</p>;
}
