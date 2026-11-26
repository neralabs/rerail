"use client";
import { useLayoutEffect, useState } from "react";
import { rpc } from "@acme/rpc-hooks";

type Post = { id: string; title: string };

// Near miss: this package does not depend on TanStack Query or tRPC, so `rpc.post.list.useQuery()` is another
// library's hook with its own cache rules.
export function AdminPostFeed() {
  const { data } = rpc.post.list.useQuery({ limit: 20 });
  const [posts, setPosts] = useState<Post[]>([]);

  useLayoutEffect(() => {
    if (data) setPosts(data.items);
  }, [data]);

  return <ul>{posts.map((p) => <li key={p.id}>{p.title}</li>)}</ul>;
}
