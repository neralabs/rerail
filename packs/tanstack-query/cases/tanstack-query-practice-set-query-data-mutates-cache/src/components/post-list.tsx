"use client";

import { api } from "@/trpc/react";
import { useChart } from "@/components/chart";

export function PostList() {
  const utils = api.useUtils();
  const chart = useChart();
  const { data } = api.post.list.useQuery();

  const like = api.post.like.useMutation({
    onSuccess: (post) => {
      utils.post.list.setData(undefined, (old) => {
        if (!old) return old;
        for (const item of old.items) {
          if (item.id === post.id) item.likes++; // caught
        }
        return old;
      });
      utils.post.byId.setData({ id: post.id }, (old) => (old ? { ...old, likes: post.likes } : old));
    },
  });

  const remove = api.post.remove.useMutation({
    onMutate: ({ id }) => {
      const previous = utils.post.list.getData();
      utils.post.list.setData(undefined, (old) =>
        old ? { ...old, items: old.items.filter((p) => p.id !== id) } : old,
      );
      return { previous };
    },
    onSettled: () => utils.post.list.invalidate(),
  });

  chart.series.setData(data?.items.map((p) => ({ x: p.createdAt, y: p.likes })) ?? []);

  return (
    <ul>
      {data?.items.map((p) => (
        <li key={p.id}>
          {p.title}
          <button onClick={() => like.mutate({ id: p.id })}>Like</button>
          <button onClick={() => remove.mutate({ id: p.id })}>Delete</button>
        </li>
      ))}
    </ul>
  );
}
