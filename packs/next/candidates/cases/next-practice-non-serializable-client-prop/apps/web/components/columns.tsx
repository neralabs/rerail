export function renderStatus(status: string) {
  return <span className={`badge badge-${status}`}>{status}</span>;
}
