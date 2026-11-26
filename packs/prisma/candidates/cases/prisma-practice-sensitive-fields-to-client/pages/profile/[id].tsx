import type { GetServerSideProps } from "next";
import { prisma } from "../../src/lib/db";

export const getServerSideProps: GetServerSideProps = async ({ params }) => {
  const user = await prisma.user.findUnique({ where: { id: String(params?.id) } }); // caught
  if (!user) return { notFound: true };
  return { props: { user } };
};

export default function Profile({ user }: { user: { name: string | null } }) {
  return <h1>{user.name}</h1>;
}
