export default async function getBoard() {
  const res = await fetch("/api/board"); // caught
  return res.json();
}
