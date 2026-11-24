"use client";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { getFeed, getFeedPage } from "@/lib/api";

// A plain and an infinite query on one key: tanstack-query-practice-infinite-and-plain-share-key reports those.
export function Feed() {
  const pages = useInfiniteQuery({
    queryKey: ["feed"],
    queryFn: ({ pageParam }) => getFeedPage(pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.next,
  });
  const head = useQuery({ queryKey: ["feed"], queryFn: () => getFeed() });
  return <p>{pages.data?.pages.length ?? head.data?.items.length}</p>;
}
