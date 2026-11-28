export function formatPrice(cents: number) {
  return `$${(cents / 100).toFixed(2)}`;
}

export const slugify = (s: string) => s.toLowerCase().replace(/\s+/g, '-');
