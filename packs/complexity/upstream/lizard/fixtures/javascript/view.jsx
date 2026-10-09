// A .jsx component: arrow functions with JSX are scored like any other function.

export const StatusBadge = ({ status, count }) => {
  if (status === "error") {
    return <span className="error">{count > 0 ? count : "!"}</span>;
  }
  if (status === "warning") {
    return <span className="warning">{count}</span>;
  }
  if (status === "ok" && count === 0) {
    return <span className="ok" />;
  }
  return <span>{status === "idle" || status === "unknown" ? "-" : status}</span>;
};

export function Simple({ label }) {
  return <span>{label}</span>;
}
