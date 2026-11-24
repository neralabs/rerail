export default async function MembersPage({ params }: { params: Promise<{ orgId: string }> }) {
  const { orgId } = await params;
  return <h1>Members of {orgId}</h1>;
}
