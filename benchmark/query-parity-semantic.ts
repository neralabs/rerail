export const semanticWorkloads = ["resolves", "usage"] as const;
export type SemanticWorkload = (typeof semanticWorkloads)[number];

export function isSemanticWorkload(workload: string): workload is SemanticWorkload {
  return (semanticWorkloads as readonly string[]).includes(workload);
}
