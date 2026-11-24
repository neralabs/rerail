"use client";
import { useState } from "react";
import { postsApi } from "@/lib/rtk";

// RTK Query's hooks are another cache.
export function RtkPosts() {
  const { data } = postsApi.endpoints.getPosts.useQuery();
  const [posts] = useState(data ?? []);
  return <p>{posts.length}</p>;
}
