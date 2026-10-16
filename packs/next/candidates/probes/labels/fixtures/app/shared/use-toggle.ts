import { useReducer } from "react";

export function useToggle() {
  const [on] = useReducer((value: boolean) => !value, false);
  return on;
}
