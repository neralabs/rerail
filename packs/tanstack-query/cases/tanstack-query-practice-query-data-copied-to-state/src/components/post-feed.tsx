"use client";
import { useLayoutEffect, useState } from "react";
import { api } from "@/lib/trpc";

type Post = { id: string; title: string };

export function PostFeed() {
  const { data } = api.post.list.useQuery({ limit: 20 });
  const [posts, setPosts] = useState<Post[]>([]);

  useLayoutEffect(() => {
    if (data) setPosts(data.items); // caught
  }, [data]);

  return <ul>{posts.map((p) => <li key={p.id}>{p.title}</li>)}</ul>;
}

export function StatsPanel() {
  const [stats] = api.stats.summary.useSuspenseQuery();
  const [snapshot] = useState(stats); // caught
  return <p>{snapshot.total}</p>;
}
