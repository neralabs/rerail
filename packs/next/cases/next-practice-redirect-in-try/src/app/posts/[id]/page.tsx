import { notFound, permanentRedirect } from "next/navigation";
import { prisma } from "@/lib/db";
import { requireUser } from "@/lib/auth";

async function getPost(id: string) {
  return prisma.post.findUnique({ where: { id } });
}

export default async function PostPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  try {
    const post = await getPost(id);
    if (!post) notFound(); // caught
    if (post.slug !== id) permanentRedirect(`/posts/${post.slug}`); // caught
    const related = await fetch(`https://api.example.com/related/${post.id}`).then((res) => {
      if (res.status === 404) notFound(); // caught
      return res.json();
    });
    return <article>{post.title} ({related.length})</article>;
  } catch (err) {
    console.error(err);
    return <p>Something went wrong.</p>;
  }
}

export async function generateMetadata({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  let post;
  try {
    post = await getPost(id);
  } catch {
    post = null;
  }
  if (!post) notFound();
  return { title: post.title };
}

export async function DraftBanner() {
  try {
    const user = await requireUser(); // cross-lane: missed until the engine joins .ts and .tsx
    return <p>Drafts by {user.name}</p>;
  } catch {
    return null;
  }
}
