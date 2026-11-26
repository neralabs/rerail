export default async function AdminHome() {
  const response = await fetch(`${process.env.NEXT_PUBLIC_ADMIN_API_URL}/stats`, {
    headers: { authorization: `Bearer ${process.env.ADMIN_API_KEY}` },
    cache: "no-store",
  });
  const stats: { users: number } = await response.json();
  return <h1>{stats.users} users</h1>;
}
