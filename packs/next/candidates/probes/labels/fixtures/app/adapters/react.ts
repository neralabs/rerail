import { useState } from "react";

export function useCounter() {
  const [count] = useState(0);
  return count;
}
