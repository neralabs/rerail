import { useEffect, useState } from "react";
import * as React from "react";

export function useCounter(initial: number) {
  const [count, setCount] = useState(initial);
  useEffect(() => {
    document.title = String(count);
  }, [count]);
  return [count, setCount] as const;
}

export const useToggle = () => React.useState(false);

function user() {
  return useless();
}

export function Panel() {
  const [value] = useCounter(1);
  const theme = React.use(ThemeContext);
  return <div>{value}{theme}</div>;
}

const ThemeContext = React.createContext("light");

export { user };
