import { QueryClient } from "@tanstack/react-query";

// A comment that names a guard is not one.
export const sharedClient = /* only read where typeof window !== "undefined" */ new QueryClient(); // caught
