import { QueryClient } from "@tanstack/react-query";

export const browserClient = typeof window === "undefined" ? undefined : new QueryClient();
