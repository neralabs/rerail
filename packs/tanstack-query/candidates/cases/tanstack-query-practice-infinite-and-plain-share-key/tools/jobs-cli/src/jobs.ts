// Near miss: this package does not depend on TanStack Query. Its own client's fetchQuery and an options object with
// a page param share a key shape, but no TanStack cache holds them.
declare const jobsClient: { fetchQuery(o: { queryKey: string[] }): Promise<unknown> };

export const jobPager = {
  queryKey: ['jobs'],
  getNextPageParam: (last: { next?: number }) => last.next,
};

export const firstJobs = () => jobsClient.fetchQuery({ queryKey: ['jobs'] });
