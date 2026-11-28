import { getPosts } from "@/lib/data";

export default async function BlogPage() {
  const posts = await getPosts();
  return <ul>{posts.map((p) => <li key={p.id}>{p.title}</li>)}</ul>;
}
