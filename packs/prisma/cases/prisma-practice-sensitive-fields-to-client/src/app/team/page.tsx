import { prisma } from "@/lib/db";
import { MemberList } from "@/components/member-list";
import Avatar from "./avatar";
import { ServerList } from "./server-list";

export default async function TeamPage() {
  const members = await prisma.user.findMany({ where: { name: { not: null } } }); // caught
  const owner = await prisma.user.findFirst(); // caught
  const admins = await prisma.user.findMany({ where: { email: { endsWith: "@example.com" } } });
  const named = await prisma.user.findMany({ select: { id: true, name: true } });
  return (
    <main>
      <Avatar user={owner} />
      <MemberList members={members} />
      <MemberList members={named} />
      <ServerList users={admins} />
    </main>
  );
}
