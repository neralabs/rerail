// An ES module file (.mjs).

export async function loadOverLimit(urls) {
  const results = [];
  for (const url of urls) {
    if (url.startsWith("http:") || url.startsWith("https:")) {
      try {
        results.push(await fetch(url));
      } catch (error) {
        results.push(error instanceof TypeError ? null : error);
      }
    } else if (url === "") {
      continue;
    }
  }
  return results;
}

export const ready = (state) => state === "ready";
