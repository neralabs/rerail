import { useState } from "react";

export const dynamic = "force-dynamic";
export const revalidate = 60;

export default function Page() {
  const [count] = useState(0);
  return <p>{count}</p>;
}
