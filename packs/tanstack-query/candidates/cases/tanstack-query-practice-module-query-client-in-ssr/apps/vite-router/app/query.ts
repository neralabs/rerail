import { QueryClient } from "@tanstack/react-query";

// Near miss: the same shapes as the React Router framework-mode app, in a Vite SPA with React Router's data
// router; its loaders run in the browser, so one client per tab is right.
export const queryClient = new QueryClient();
