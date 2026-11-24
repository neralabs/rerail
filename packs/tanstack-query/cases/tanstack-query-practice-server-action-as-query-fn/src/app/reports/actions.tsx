'use server'

import { db } from '@/lib/db'

export async function getReportSummary(projectId: string) {
  const runs = await db.run.count({ where: { projectId } })
  return <p>{runs} runs</p>
}

export async function refreshReport(projectId: string) {
  await db.report.update({ where: { projectId }, data: { refreshedAt: new Date() } })
}
