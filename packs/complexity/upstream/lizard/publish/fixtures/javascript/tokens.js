export function isAbsolute(url) {
  return /^([a-z][a-z\d+\-.]*:)?\/\//i.test(url);
}

export function protocolOf(url) {
  return url.split(':')[0]?.toLowerCase();
}

export function firstName(lookup) {
  return lookup()?.name;
}

export function keepWanted(items, wanted) {
  return items.filter((item) =>
    item.ok && (item.id === wanted ? true : item.fallback));
}
