import { unstable_cache } from "next/cache";
import { cacheTag } from "next/cache";
import { db } from "./db";

export async function getPosts() {
  const res = await fetch("https://cms.example.com/posts", { next: { tags: ["posts"] } });
  return (await res.json()) as { id: string; title: string }[];
}

export const getPost = (slug: string) =>
  unstable_cache(() => db.post.findUnique({ where: { slug } }), ["post", slug], { tags: [`post-${slug}`] })();

export async function getDashboard() {
  "use cache";
  cacheTag("dashboard");
  return db.stats.findFirst();
}
