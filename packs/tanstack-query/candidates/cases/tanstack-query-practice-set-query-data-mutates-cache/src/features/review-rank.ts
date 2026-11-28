import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";

type Version = { id: number; rank: { ratingCount: number } };
type Model = { id: number; rank: { ratingCount: number }; modelVersions: Version[] };
type SecretVersion = { id: string; createdAt: string };

declare function getModel(id: number): Promise<Model>;
declare function getVersions(id: string): Promise<SecretVersion[]>;

// civitai's review updater: several changes in one updater are one fix, reported once at the first.
export function useRateModel(id: number) {
  const queryClient = useQueryClient();
  return () =>
    queryClient.setQueryData<Model>(["model", id], (old) => {
      if (!old) return old;
      old.rank.ratingCount += 1; // caught
      old.modelVersions = old.modelVersions.map((v) => {
        v.rank.ratingCount += 1;
        return v;
      });
      return old;
    });
}

// Infisical's secret versions: `select` receives the cached array itself, so `sort` reorders the cache.
export function useSecretVersions(secretId: string) {
  return useQuery({
    queryKey: ["secret-versions", secretId],
    queryFn: () => getVersions(secretId),
    select: useCallback((data: SecretVersion[]) => {
      return data.sort((a, b) => b.createdAt.localeCompare(a.createdAt)); // caught
    }, []),
  });
}

export function useSecretVersionsInline(secretId: string) {
  return useQuery({
    queryKey: ["secret-versions", secretId, "inline"],
    queryFn: () => getVersions(secretId),
    select: (data) => data.reverse(), // caught
  });
}

// Near misses: a copy sorted in select, toSorted, and a `select` that is not a query option.
export function useSortedCopies(secretId: string) {
  useQuery({
    queryKey: ["secret-versions", secretId, "copy"],
    queryFn: () => getVersions(secretId),
    select: (data) => [...data].sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
  });
  useQuery({
    queryKey: ["secret-versions", secretId, "to-sorted"],
    queryFn: () => getVersions(secretId),
    select: (data) => data.toSorted((a, b) => b.createdAt.localeCompare(a.createdAt)),
  });
  const picker = { options: [3, 1, 2], select: (xs: number[]) => xs.sort() };
  return picker;
}
