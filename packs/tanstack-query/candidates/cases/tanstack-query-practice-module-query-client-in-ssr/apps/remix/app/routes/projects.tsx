import { useLoaderData } from "react-router";
import { queryClient } from "../query";

// The loader fills the app's client on the server, so a module-scope client there is shared between requests.
export async function loader() {
  return queryClient.ensureQueryData({ queryKey: ["projects"], queryFn: () => fetch("/api/projects").then((r) => r.json()) });
}

export default function Projects() {
  const projects = useLoaderData<typeof loader>();
  return <pre>{JSON.stringify(projects)}</pre>;
}
