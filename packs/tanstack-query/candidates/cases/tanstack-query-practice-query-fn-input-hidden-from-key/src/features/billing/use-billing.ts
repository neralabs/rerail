import { useQuery } from '@tanstack/react-query';
import { billingKeys } from '#/features/billing/keys';

declare function get(...args: unknown[]): Promise<unknown>;

// The factory comes through the `#/` alias.
export function useBilling(orgId: string, authHeader: string, month: string) {
  // A request header the factory leaves out of the key is right: not reported.
  useQuery({ queryKey: billingKeys.invoices(orgId, authHeader), queryFn: () => get(orgId, authHeader) });
  useQuery({ queryKey: billingKeys.usage(orgId, month), queryFn: () => get(orgId, month) }); // caught
}
