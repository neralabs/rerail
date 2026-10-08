import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode, useCallback, useState } from "react";
import { createRoot } from "react-dom/client";
import { ThemeProvider } from "next-themes";

import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";

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

createRoot(root).render(
  <StrictMode>
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem>
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <PlaygroundRoot />
          <Toaster />
        </TooltipProvider>
      </QueryClientProvider>
    </ThemeProvider>
  </StrictMode>,
);
