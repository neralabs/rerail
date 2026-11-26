"use client";

import { readFileSync } from "fs";

// The same shape in a React Router app that has no next dependency: not this rule's.
export function Uploader() {
  return <pre>{String(readFileSync)}</pre>;
}
