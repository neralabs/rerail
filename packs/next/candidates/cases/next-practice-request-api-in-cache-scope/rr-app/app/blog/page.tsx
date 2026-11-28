"use cache";

import { cookies } from "next/headers";

// The same shapes in a React Router app that has no next dependency: not this rule's.
export default async function BlogPage({ searchParams }: { searchParams: Promise<{ page?: string }> }) {
  const { page } = await searchParams;
  const theme = (await cookies()).get("theme");
  return <p>{page} {theme?.value}</p>;
}
