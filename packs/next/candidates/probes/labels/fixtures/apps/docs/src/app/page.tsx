import { useState } from "react";

export default function DocsPage() {
  const [query] = useState("");
  return <input defaultValue={query} />;
}
