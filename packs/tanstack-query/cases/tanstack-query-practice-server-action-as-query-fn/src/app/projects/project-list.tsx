'use client'

import { useMutation, useQuery } from '@tanstack/react-query'
import { createProject, getProject, getProjects } from './actions'
import { fetchProjectsFromApi } from '@/lib/api'

export function ProjectList({ ids }: { ids: string[] }) {
  // cross-lane: missed until the engine joins .ts and .tsx
  const projects = useQuery({ queryKey: ['projects'], queryFn: getProjects })
  // cross-lane: missed until the engine joins .ts and .tsx
  const first = useQuery({ queryKey: ['project', ids[0]], queryFn: () => getProject(ids[0]) })
  const viaApi = useQuery({ queryKey: ['projects', 'api'], queryFn: fetchProjectsFromApi })
  const viaFetch = useQuery({
    queryKey: ['projects', 'fetch'],
    queryFn: ({ signal }) => fetch('/api/projects', { signal }).then((r) => r.json()),
  })
  const create = useMutation({ mutationFn: createProject })
  const rename = useMutation({ mutationFn: (name: string) => createProject(name) })

  return (
    <ul>
      {projects.data?.map((p) => <li key={p.id}>{p.name}</li>)}
      <li>{first.data?.name}</li>
      <li>{viaApi.data?.length}</li>
      <li>{viaFetch.data?.length}</li>
      <button onClick={() => create.mutate('New')}>New</button>
      <button onClick={() => rename.mutate('Renamed')}>Rename</button>
    </ul>
  )
}
