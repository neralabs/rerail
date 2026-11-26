import { useQuery } from '@tanstack/react-query'
import { countInbox } from '@/lib/api'

// ['inbox'] is INBOX_KEY, the infinite inbox list's key.
export const useInboxCount = () => useQuery({ queryKey: ['inbox'], queryFn: countInbox })
