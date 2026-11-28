import { useState } from "react";
import type { Todo } from "@/features/todos";

// Local state, not a query: the rows the user saved last.
export function useSavedFilter() {
  const [saved] = useState<Todo[]>([]);
  return saved;
}
