export const billingKeys = {
  invoices: (orgId: string, authHeader: string) => ['billing', 'invoices', orgId] as const,
  usage: (orgId: string, month: string) => ['billing', 'usage', orgId] as const,
};
