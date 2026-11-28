"use client";

import { useTransition } from "react";
import { refreshComments } from "./refresh";

export function Refresh() {
  const [, start] = useTransition();
  return <button onClick={() => start(() => refreshComments())}>Refresh</button>;
}
