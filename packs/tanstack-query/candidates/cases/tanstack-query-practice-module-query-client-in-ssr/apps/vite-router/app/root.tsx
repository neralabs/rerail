import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Outlet } from "react-router";

const queryClient = new QueryClient(); // near miss: a Vite SPA; the app/ folder is not Next.js or Remix

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <Outlet />
    </QueryClientProvider>
  );
}
