// Optional parameters, optional properties and optional chaining are not decisions.

export interface Settings {
  name?: string;
  depth?: number;
  nested?: { limit?: number };
}

export function optionalAtLimit(settings?: Settings, fallback?: number): number {
  if (settings === undefined) {
    return fallback === undefined ? 0 : fallback;
  }
  if (settings.name !== undefined) {
    return settings.name.length;
  }
  const limit = settings.nested?.limit;
  if (limit !== undefined) {
    return limit;
  }
  return settings?.depth !== undefined ? settings.depth : -1;
}
