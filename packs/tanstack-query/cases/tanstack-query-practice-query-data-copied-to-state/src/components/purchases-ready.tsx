import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";

declare function identifyPurchasesUser(apiKey: string, userId: string): Promise<void>;
declare function getMe(): Promise<{ id: string }>;

// The state records that an external call finished for this user (karakeep's purchases gate): not a copy.
export function usePurchases(apiKey: string) {
  const { data: user } = useQuery({ queryKey: ["me"], queryFn: getMe });
  const userId = user?.id;
  const [readyUserId, setReadyUserId] = useState<string | null>(null);

  useEffect(() => {
    if (!userId) return;
    let cancelled = false;
    identifyPurchasesUser(apiKey, userId).then(() => {
      if (!cancelled) setReadyUserId(userId);
    });
    return () => {
      cancelled = true;
    };
  }, [apiKey, userId]);

  return { isAvailable: !!userId && readyUserId === userId };
}
