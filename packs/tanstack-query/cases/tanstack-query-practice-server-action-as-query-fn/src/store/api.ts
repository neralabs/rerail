import { createApi, fakeBaseQuery } from '@reduxjs/toolkit/query/react'
import { getProjects } from '@/app/projects/actions'

export const projectsApi = createApi({
  baseQuery: fakeBaseQuery(),
  endpoints: (build) => ({
    projects: build.query({
      queryFn: async () => ({ data: await getProjects() }),
    }),
  }),
})
