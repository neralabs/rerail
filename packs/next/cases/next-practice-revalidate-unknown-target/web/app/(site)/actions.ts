"use server";

import { revalidatePath, revalidateTag, updateTag } from "next/cache";
import * as nextCache from "next/cache";
import { revalidatePath as refreshPath } from "next/cache";
import { db } from "@/lib/db";

const BLOG = "/blog";

export async function publishPost(slug: string, title: string, orgId: string, id: string) {
  await db.post.update({ where: { slug }, data: { title, published: true } });

  revalidateTag("post", "max"); // a tag: next-practice-revalidate-unknown-tag's, not this rule's
  revalidatePath("/dashbaord"); // caught
  revalidatePath("/blog/[slug]"); // caught
  revalidatePath("/dashboard/page"); // caught
  revalidatePath(`/organisations/${orgId}/members`); // caught
  updateTag(`comment-${id}`); // a tag: next-practice-revalidate-unknown-tag's
  nextCache.revalidatePath("/(marketing)/blog"); // caught
  refreshPath("/blog/[slug]/comments", "page"); // caught
  revalidatePath(`/orgs/[orgId]/members/${id}`); // caught
  revalidatePath("/(admin)/settings"); // caught

  revalidateTag("posts", "max");
  revalidateTag(`post-${slug}`, "max");
  updateTag("dashboard");
  revalidatePath("/dashboard");
  revalidatePath("/blog/[slug]", "page");
  revalidatePath("/(site)/blog/[slug]", "page");
  revalidatePath("/blog/[slug]", "layout");
  revalidatePath("/", "layout");
  revalidatePath("/");
  revalidatePath(`/orgs/${orgId}/members`);
  revalidatePath(`/blog/${slug}`);
  revalidatePath("/docs");
  revalidatePath("/docs/getting-started/install");
  revalidatePath("/api/posts");
  revalidatePath("/blog/");
  revalidatePath(BLOG);
  revalidatePath("/test");
  revalidatePath("/blog/page");
  revalidatePath("/orgs", "layout");
  revalidatePath(`${BLOG}/archive`);
  revalidatePath(`/blog${slug ? `/${slug}` : ""}`);
  nextCache.revalidatePath("/blog");
  revalidatePath("/acme/settings");
}

export async function refreshAny(path: string, type: "page" | "layout") {
  revalidatePath(path);
  revalidatePath(path, type);
  revalidatePath("/blog/[slug]", type);
}
