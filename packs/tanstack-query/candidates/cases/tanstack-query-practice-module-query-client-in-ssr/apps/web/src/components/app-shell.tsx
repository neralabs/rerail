"use client";

import { QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { makeShellClient } from "@/components/make-shell-client";

const shellClient = makeShellClient(); // caught

export function AppShell({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={shellClient}>{children}</QueryClientProvider>;
}
