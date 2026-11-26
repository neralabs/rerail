import { queryClient } from "../query";

export const route = {
  preload: () => queryClient.prefetchQuery({ queryKey: ["feed"], queryFn: () => fetch("/api/feed").then((r) => r.json()) }),
};

export default function Home() {
  return <main />;
}
