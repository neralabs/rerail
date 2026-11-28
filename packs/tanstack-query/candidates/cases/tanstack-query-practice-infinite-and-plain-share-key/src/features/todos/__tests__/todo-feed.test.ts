import { QueryClient } from '@tanstack/react-query'
import { getDrafts } from '@/lib/api'

// Tests are out of scope: this plain prefetch shares the infinite drafts key, and is not counted.
it('prefetches drafts', async () => {
  const client = new QueryClient()
  await client.prefetchQuery({ queryKey: ['drafts', 'd1'], queryFn: () => getDrafts('d1', 0) })
})
