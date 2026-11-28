'use client'

import { useMutation, useQuery } from '@tanstack/react-query'
import { getReportSummary, refreshReport } from './actions'

export function ReportSummary({ projectId }: { projectId: string }) {
  const { data } = useQuery({
    queryKey: ['report-summary', projectId],
    queryFn: () => getReportSummary(projectId), // caught
  })
  const refresh = useMutation({ mutationFn: refreshReport })
  return (
    <section>
      {data}
      <button onClick={() => refresh.mutate(projectId)}>Refresh</button>
    </section>
  )
}
