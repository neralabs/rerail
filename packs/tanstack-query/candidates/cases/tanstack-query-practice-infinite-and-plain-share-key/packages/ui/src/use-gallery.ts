import { useInfiniteQuery } from '@tanstack/react-query'

// Published as @todo/ui-kit from the folder packages/ui: the web app imports it by that name and shares its client.
export const useGallery = (albumId: string, load: (albumId: string, cursor: number) => Promise<{ nextCursor: number | null }>) =>
  useInfiniteQuery({
    queryKey: ['gallery', albumId], // caught
    queryFn: ({ pageParam }) => load(albumId, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
