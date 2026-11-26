"use client";

import { useEffect, useState } from "react";
import type { ReportRow } from "@/server/reports";
import { type ReportFilter } from "@/server/reports";
import { ReportSummary } from "@/server/reports";

export function useReport(filter: ReportFilter) {
  const [rows, setRows] = useState<ReportRow[]>([]);
  const [summary, setSummary] = useState<ReportSummary | null>(null);
  useEffect(() => {
    const q = new URLSearchParams(filter);
    fetch(`/api/reports?${q}`)
      .then((r) => r.json())
      .then((body: { rows: ReportRow[]; summary: ReportSummary }) => {
        setRows(body.rows);
        setSummary(body.summary);
      });
  }, [filter]);
  return { rows, summary };
}
