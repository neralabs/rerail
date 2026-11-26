import { useLoaderData } from "react-router";
import { queryClient } from "../query";

// A data-router loader: it runs in the browser in this app.
export async function loader() {
  return queryClient.ensureQueryData({ queryKey: ["projects"], queryFn: () => fetch("/api/projects").then((r) => r.json()) });
}

export default function Projects() {
  const projects = useLoaderData<typeof loader>();
  return <pre>{JSON.stringify(projects)}</pre>;
}
