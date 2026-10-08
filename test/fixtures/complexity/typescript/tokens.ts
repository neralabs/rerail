export function hasScheme(url: string): boolean {
  return /^([a-z]+:)?\/\//i.test(url);
}

export function lowerHost(url: string): string | undefined {
  return url.split("/")[2]?.toLowerCase();
}

export function labelOf(source: Map<string, { label: string }>) {
  return source.get("first")?.label;
}

export function chooseFirst(values: Array<{ ok: boolean; id: string; spare: boolean }>, wanted: string) {
  return values.filter((value) =>
    value.ok && (value.id === wanted ? true : value.spare));
}
