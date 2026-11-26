export default async function CommentsPage() {
  const res = await fetch("https://api.example.com/comments", { next: { tags: ["comments"] } });
  const comments: { id: string; body: string }[] = await res.json();
  return <ul>{comments.map((c) => <li key={c.id}>{c.body}</li>)}</ul>;
}
