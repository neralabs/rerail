import { useQueryClient, type InfiniteData } from "@tanstack/react-query";
import type { FeedPage, FeedItem, Draft } from "@/lib/api/feed";

const feedKey = ["feed"] as const;

export function useFeedCache() {
  const queryClient = useQueryClient();

  const prepend = (item: FeedItem) =>
    queryClient.setQueryData<InfiniteData<FeedPage>>(feedKey, (old) => {
      if (!old) return old;
      old.pages.forEach((page, i) => {
        if (i === 0) page.items.unshift(item); // caught
      });
      return { ...old };
    });

  const discardDraft = () =>
    queryClient.setQueryData<Draft>(["draft"], (old) => {
      if (!old) return old;
      delete old.body; // caught
      return { ...old };
    });

  const patchDraft = (patch: Partial<Draft>) => {
    const draft = queryClient.getQueryData(["draft"]) as Draft;
    Object.assign(draft, patch); // caught
  };

  // Near misses: a new pages array, a copy of the draft, a fresh Map.
  const prependImmutable = (item: FeedItem) =>
    queryClient.setQueryData<InfiniteData<FeedPage>>(feedKey, (old) =>
      old
        ? {
            ...old,
            pages: old.pages.map((page, i) => (i === 0 ? { ...page, items: [item, ...page.items] } : page)),
          }
        : old,
    );

  const patchDraftCopy = (patch: Partial<Draft>) => {
    const draft = queryClient.getQueryData<Draft>(["draft"]);
    queryClient.setQueryData(["draft"], Object.assign({}, draft, patch));
  };

  const markSeen = (id: string) =>
    queryClient.setQueryData<Map<string, boolean>>(["seen"], (old) => {
      const next = new Map(old);
      next.set(id, true);
      return next;
    });

  return { prepend, discardDraft, patchDraft, prependImmutable, patchDraftCopy, markSeen };
}
