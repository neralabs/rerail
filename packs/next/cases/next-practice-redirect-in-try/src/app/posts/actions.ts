"use server";

import { revalidatePath } from "next/cache";
import { notFound, redirect, unstable_rethrow } from "next/navigation";
import { isRedirectError } from "next/dist/client/components/redirect-error";
import { z } from "zod";
import { prisma } from "@/lib/db";
import { requireUser, requireAdmin, currentUser } from "@/lib/auth";

const PostInput = z.object({ title: z.string().min(1), body: z.string() });

export async function createPost(formData: FormData) {
  try {
    const input = PostInput.parse(Object.fromEntries(formData));
    const post = await prisma.post.create({ data: input });
    revalidatePath("/posts");
    redirect(`/posts/${post.id}`); // caught
  } catch (error) {
    return { error: "Could not create the post" };
  }
}

export async function publishPost(id: string) {
  try {
    const user = await requireUser(); // caught
    await prisma.post.update({ where: { id, authorId: user.id }, data: { published: true } });
  } catch {
    return { error: "Could not publish" };
  }
  revalidatePath(`/posts/${id}`);
}

export async function deleteAllPosts() {
  try {
    await requireAdmin(); // caught
    await prisma.post.deleteMany();
  } catch (error) {
    console.error(error);
    throw new Error("Delete failed");
  }
}

export async function renamePost(id: string, title: string) {
  try {
    try {
      await prisma.post.update({ where: { id }, data: { title } });
      redirect(`/posts/${id}`); // caught
    } catch (inner) {
      throw inner;
    }
  } catch (outer) {
    return { error: String(outer) };
  }
}

export async function importPosts(rows: { title: string; body: string }[]) {
  try {
    await prisma.$transaction(async (tx) => {
      for (const row of rows) {
        if (!row.title) redirect("/posts/import?error=title"); // caught
        await tx.post.create({ data: row });
      }
    });
  } catch (error) {
    return { error: "Import failed" };
  }
}

// Near misses below: none of these is reported.

export async function deletePost(id: string) {
  try {
    await prisma.post.delete({ where: { id } });
    redirect("/posts");
  } catch (error) {
    unstable_rethrow(error);
    return { error: "Could not delete" };
  }
}

export async function updatePost(id: string, formData: FormData) {
  try {
    const input = PostInput.parse(Object.fromEntries(formData));
    await prisma.post.update({ where: { id }, data: input });
    redirect(`/posts/${id}`);
  } catch (error) {
    if (isRedirectError(error)) throw error;
    return { error: "Could not update" };
  }
}

export async function duplicatePost(id: string) {
  try {
    const copy = await prisma.post.create({ data: { title: "Copy", body: "" } });
    redirect(`/posts/${copy.id}?from=${id}`);
  } catch (error) {
    if (error instanceof z.ZodError) {
      return { error: error.message };
    }
    throw error;
  }
}

export async function archivePost(id: string) {
  try {
    await prisma.post.update({ where: { id }, data: { archived: true } });
  } catch (error) {
    return { error: "Could not archive" };
  }
  redirect("/posts");
}

export async function restorePost(id: string) {
  try {
    await prisma.post.update({ where: { id }, data: { archived: false } });
    redirect(`/posts/${id}`);
  } finally {
    revalidatePath("/posts");
  }
}

export async function retryPost(id: string) {
  try {
    await prisma.post.update({ where: { id }, data: { retries: { increment: 1 } } });
  } catch {
    redirect("/posts?error=retry");
  }
}

export async function scheduleRedirect(id: string, register: (go: () => void) => void) {
  try {
    const go = () => redirect(`/posts/${id}`);
    register(go);
  } catch {
    return { error: "Could not schedule" };
  }
}

export async function viewerName() {
  try {
    const user = await currentUser();
    return user?.name ?? "guest";
  } catch {
    return "guest";
  }
}

// Reported below: NEXT_REDIRECT is an Error, so the guarded return swallows it before the rethrow.
export async function movePost(id: string, category: string) {
  try {
    await prisma.post.update({ where: { id }, data: { category } });
    redirect(`/posts/${id}`); // caught
  } catch (error) {
    if (error instanceof Error) return { error: error.message };
    throw error;
  }
}

export async function copyPost(id: string) {
  try {
    const copy = await prisma.post.create({ data: { title: id, body: "" } });
    redirect(`/posts/${copy.id}`); // caught
  } catch (error) {
    if (!(error instanceof Error)) throw error;
    return { error: error.message };
  }
}

// The chain's .catch callback swallows the rejection the redirect makes.
export async function pinPost(id: string) {
  await prisma.post
    .update({ where: { id }, data: { pinned: true } })
    .then(() => redirect(`/posts/${id}`)) // caught
    .catch(() => null);
}

// Near misses: the .catch callback rethrows; the catch ends by throwing the same 404 again.
export async function unpinPost(id: string) {
  await prisma.post
    .update({ where: { id }, data: { pinned: false } })
    .then(() => redirect(`/posts/${id}`))
    .catch((error) => {
      console.error(error);
      throw error;
    });
}

export async function showDraft(id: string) {
  try {
    const draft = await prisma.post.findUnique({ where: { id } });
    if (!draft) notFound();
  } catch {
    notFound();
  }
}
