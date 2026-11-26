import { queryOptions } from '@tanstack/react-query'
import { getBoard } from './actions'

// Near miss: this package does not depend on Next.js.
export const boardQuery = (id: string) =>
  queryOptions({
    queryKey: ['board', id],
    queryFn: () => getBoard(id),
  })
