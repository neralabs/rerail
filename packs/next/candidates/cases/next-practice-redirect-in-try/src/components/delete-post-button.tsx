"use client";

import { redirect, useRouter } from "next/navigation";
import { useState } from "react";
import { toast } from "sonner";
import { deletePost } from "@/app/posts/actions";

export function DeletePostButton({ id }: { id: string }) {
  const [busy, setBusy] = useState(false);
  const router = useRouter();

  const onDelete = async () => {
    setBusy(true);
    try {
      await deletePost(id);
      toast.success("Post deleted");
      redirect("/posts"); // caught
    } catch {
      toast.error("Could not delete the post");
    } finally {
      setBusy(false);
    }
  };

  const onArchive = async () => {
    try {
      await deletePost(id);
      router.push("/posts");
    } catch {
      toast.error("Could not archive the post");
    }
  };

  return (
    <>
      <button disabled={busy} onClick={onDelete}>Delete</button>
      <button onClick={onArchive}>Archive</button>
    </>
  );
}
