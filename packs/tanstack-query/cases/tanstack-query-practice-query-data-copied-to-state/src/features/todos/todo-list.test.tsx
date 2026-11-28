import { useEffect, useState } from "react";
import { useTodos, type Todo } from "@/features/todos";

// Tests are out of scope.
export function Harness() {
  const { data } = useTodos();
  const [todos, setTodos] = useState<Todo[]>([]);
  useEffect(() => {
    if (data) setTodos(data);
  }, [data]);
  return <pre>{JSON.stringify(todos)}</pre>;
}
