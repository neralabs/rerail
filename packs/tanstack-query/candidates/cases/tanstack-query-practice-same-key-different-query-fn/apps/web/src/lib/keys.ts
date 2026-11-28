export const projectKeys = {
  all: ["project"] as const,
  detail: (id: string) => ["project", id] as const,
};

export const userKeys = {
  all: ["user"] as const,
};

export const reportKeys = {
  daily: (params: object) => ["reports", "daily", params] as const,
  limits: (params: object) => ["reports", "limits", params] as const,
};
