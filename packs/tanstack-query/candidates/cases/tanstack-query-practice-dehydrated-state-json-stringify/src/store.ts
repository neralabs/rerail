const store = new Map<string, number>();

export function dehydrate() {
  return Object.fromEntries(store);
}
