"use client";

export function LegacyTable({ rows }) {
  return <table><tbody>{rows.map((r) => <tr key={r.id}><td>{r.name}</td></tr>)}</tbody></table>;
}
