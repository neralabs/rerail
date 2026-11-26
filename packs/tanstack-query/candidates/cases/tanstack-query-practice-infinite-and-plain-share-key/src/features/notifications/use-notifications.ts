import { useInfiniteQuery, useMutation, useQueryClient, type InfiniteData } from '@tanstack/react-query'
import { getNotifications, type Page } from '@/lib/api'

type Note = { id: string; text: string }

export const useNotifications = (userId: string) =>
  useInfiniteQuery({
    queryKey: ['notifications', userId],
    queryFn: ({ pageParam }) => getNotifications(userId, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })

export function usePushNotification(userId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (note: Note) => note,
    onSuccess: (note) => {
      // Writes a plain array where the infinite query keeps { pages, pageParams }.
      qc.setQueryData(['notifications', userId], (old: Note[] | undefined) => [note, ...(old ?? [])]) // caught
    },
  })
}

// Near miss: the updater keeps the infinite shape.
export function usePushNotificationFixed(userId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (note: Note) => note,
    onSuccess: (note) => {
      qc.setQueryData<InfiniteData<Page<Note>>>(['notifications', userId], (old) =>
        old && { ...old, pages: [{ ...old.pages[0], items: [note, ...old.pages[0].items] }, ...old.pages.slice(1)] },
      )
    },
  })
}

// Near miss: a named updater, whose shape this rule does not read.
export function usePushNotificationHelper(userId: string, prepend: (old: unknown) => unknown) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (note: Note) => note,
    onSuccess: () => qc.setQueryData(['notifications', userId], prepend),
  })
}
