"use client";

import { useState } from "react";
import { formatBytes } from "@/lib";

export function useUploadLabel() {
  const [size, setSize] = useState(0);
  return { label: size > 0 ? formatBytes(size) : "No file", setSize };
}
