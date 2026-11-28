"use client";
import { useEffect, useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";

type Page = { items: { id: string }[]; next?: number };

async function fetchPage(cursor: number): Promise<Page> {
  const res = await fetch(`/api/feed?cursor=${cursor}`);
  if (!res.ok) throw new Error(`feed: ${res.status}`);
  return res.json();
}

// An accumulation through an updater, and a flag set beside the data: neither copies the data.
export function InfiniteFeed() {
  const { data } = useInfiniteQuery({
    queryKey: ["feed"],
    queryFn: ({ pageParam }) => fetchPage(pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.next,
  });
  const [seen, setSeen] = useState<string[]>([]);
  const [loadedOnce, setLoadedOnce] = useState(false);

  useEffect(() => {
    const last = data?.pages.at(-1);
    if (last) setSeen((prev) => [...prev, ...last.items.map((i) => i.id)]);
    if (data) setLoadedOnce(true);
  }, [data]);

  return <p>{loadedOnce ? seen.length : "…"}</p>;
}
