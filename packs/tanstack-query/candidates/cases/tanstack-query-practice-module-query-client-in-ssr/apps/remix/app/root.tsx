import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Outlet } from "react-router";

const queryClient = new QueryClient(); // caught

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <Outlet />
    </QueryClientProvider>
  );
}
