/** Query-parity workloads that evaluate `benchmark/<name>/with` (semantic-resolution rule catalogs) over the corpus. */
export const semanticWorkloads = ["resolves", "usage"] as const;
export type SemanticWorkload = (typeof semanticWorkloads)[number];

export function isSemanticWorkload(workload: string): workload is SemanticWorkload {
  return (semanticWorkloads as readonly string[]).includes(workload);
}
