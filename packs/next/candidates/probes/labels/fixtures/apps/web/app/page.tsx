import { useState } from "react";

export default function WebPage() {
  const [open] = useState(false);
  return <p>{String(open)}</p>;
}
