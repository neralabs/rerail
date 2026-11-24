import { dehydrate, HydrationBoundary, QueryClient } from '@tanstack/react-query'
import { getReportSummary } from './actions'
import { ReportSummary } from './report-summary'

export default async function ReportPage({ params }: { params: Promise<{ projectId: string }> }) {
  const { projectId } = await params
  const queryClient = new QueryClient()
  await queryClient.prefetchQuery({
    queryKey: ['report-summary', projectId],
    queryFn: () => getReportSummary(projectId),
  })
  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <ReportSummary projectId={projectId} />
    </HydrationBoundary>
  )
}
