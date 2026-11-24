export async function decrypt(token: string | undefined) {
  if (!token) return null;
  return { userId: token.split(".")[0] };
}
