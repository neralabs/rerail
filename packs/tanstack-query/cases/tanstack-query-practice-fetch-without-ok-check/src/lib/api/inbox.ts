export async function getInbox() {
  const res = await fetch("/api/inbox"); // cross-lane: missed until the engine joins .ts and .tsx
  return res.json();
}
