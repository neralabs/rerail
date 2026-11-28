"use client";
import { useState } from "react";
import { useTodoDraft } from "@/features/todos";

// A custom hook that returns local state, not a query.
export function DraftList() {
  const { data } = useTodoDraft();
  const [first] = useState(data);
  return <p>{first.length}</p>;
}
