/** Restore the typed API's producer order from Rerail's unordered finding collection. */
export interface FactFinding {
  path: string;
  code: string;
  clauseId?: string;
  span?: { startByte: number };
  variables?: { row?: Record<string, unknown> };
}
function compare(left: unknown, right: unknown): number {
  if (typeof left === "number" && typeof right === "number") return left - right;
  return Buffer.compare(Buffer.from(String(left ?? "")), Buffer.from(String(right ?? "")));
}
export function orderFactFindings<T extends FactFinding>(contract: string | undefined, rows: T[], paths: string[] = []): T[] {
  const policy = contract?.endsWith("policy-comparison") === true;
  const memo = contract === "memoized-policy-comparison";
  const key = (finding: T): unknown[] => {
    const row = finding.variables?.row ?? {};
    if (policy) return [row.phase, memo && row.phase === 1 ? "" : row.function, row.step, row.ordinal];
    let path = finding.path;
    if (contract === "lazy-analyzed") path = (JSON.parse(String(row.payload)) as { file: string }).file;
    const raw = finding.clauseId?.startsWith("deleted-") === true;
    return [row.producer_phase ?? (raw ? 1 : 2), paths.indexOf(path), row.fact_order,
      raw ? finding.clauseId!.startsWith("deleted-cold-") ? 0 : 1 : row.diagnostic_order,
      raw ? Number(finding.clauseId!.match(/\d+$/)?.[0]) : finding.span?.startByte];
  };
  return [...rows].sort((left, right) => {
    const a = key(left), b = key(right);
    for (let i = 0; i < a.length; i++) { const order = compare(a[i], b[i]); if (order) return order; }
    return 0;
  });
}
