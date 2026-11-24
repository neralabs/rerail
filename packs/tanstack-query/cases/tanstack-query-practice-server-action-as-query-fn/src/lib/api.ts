import { getProjectCount } from '@/app/projects/actions'

export async function fetchProjectsFromApi() {
  const res = await fetch('/api/projects')
  if (!res.ok) throw new Error('Failed to load projects')
  return res.json()
}

export async function loadProjectCount() {
  const count = await getProjectCount()
  return { count }
}
