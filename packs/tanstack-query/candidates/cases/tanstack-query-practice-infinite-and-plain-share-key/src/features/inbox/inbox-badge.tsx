'use client'

import { useSuspenseQuery } from '@tanstack/react-query'
import { countInbox } from '@/lib/api'

// The unread feed's key, read as a plain query from a .tsx file.
export function InboxBadge() {
  const { data } = useSuspenseQuery({ queryKey: ['inbox', 'unread'], queryFn: countInbox })
  return <span>{data.unread}</span>
}
