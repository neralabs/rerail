import { dehydrate, HydrationBoundary, QueryClient } from '@tanstack/react-query'
import { getProjects } from './actions'
import { ProjectList } from './project-list'

export default async function ProjectsPage() {
  const queryClient = new QueryClient()
  await queryClient.prefetchQuery({ queryKey: ['projects'], queryFn: () => getProjects() })
  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <ProjectList ids={[]} />
    </HydrationBoundary>
  )
}
