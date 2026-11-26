import { useQuery } from '@tanstack/react-query'
import { useGallery } from '@todo/ui-kit'

// The same key as @todo/ui-kit's useGallery, a package this app imports by its package.json name.
export const useGalleryCount = (albumId: string, count: (albumId: string) => Promise<number>) =>
  useQuery({ queryKey: ['gallery', albumId], queryFn: () => count(albumId) })

export { useGallery }
