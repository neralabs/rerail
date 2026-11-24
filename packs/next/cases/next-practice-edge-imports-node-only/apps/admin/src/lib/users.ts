export async function findUserByEmail(email: string) {
  const res = await fetch(`${process.env.API_URL}/users?email=${encodeURIComponent(email)}`);
  return res.ok ? ((await res.json()) as { passwordHash: string }) : null;
}
