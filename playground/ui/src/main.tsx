import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode, useCallback, useState } from "react";
import { createRoot } from "react-dom/client";

import "./styles.css";
import { RerailPlaygroundContainer } from "./RerailPlaygroundContainer";

/** `#playground?ruleId=…` is what the client itself writes back while a rule is connected. */
const ruleIdFromLocation = (): string | undefined => {
  const hash = globalThis.location.hash.replace(/^#/u, "");
  const query = hash.slice(hash.indexOf("?") + 1);
  return new URLSearchParams(hash.includes("?") ? query : "").get("ruleId") ?? undefined;
};

const PlaygroundRoot = () => {
  const [ruleId, setRuleId] = useState(ruleIdFromLocation);
  // There is no other screen to go back to; Back clears the selection and stops the run.
  const onBack = useCallback(() => {
    globalThis.history.replaceState(null, "", "#playground");
    setRuleId(undefined);
  }, []);
  return <RerailPlaygroundContainer key={ruleId ?? "none"} initialRuleId={ruleId} onBack={onBack} />;
};

const root = document.getElementById("root");
if (root === null) throw new Error("Playground root element is missing");
const queryClient = new QueryClient();

// Light and dark follow the operating system, as the daemon's own page does: the token sheet
// switches on `prefers-color-scheme`, and so does Tailwind's default `dark:` variant.
createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <PlaygroundRoot />
    </QueryClientProvider>
  </StrictMode>,
);
