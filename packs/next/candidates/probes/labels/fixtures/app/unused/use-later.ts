import { useRef } from "react";

export function useLater() {
  return useRef(null);
}
