// The JSON document a `rerail-engine` command writes to stdout. Since its CLI reports each command's timing (decision
// 0288), the document is followed by lines of text ("  after the run … s wall: …"), so stdout as a whole is not JSON:
// this reads the one JSON object or array it starts with and leaves whatever follows.

/** The JSON object or array `text` starts with (after white space), whatever text follows it; typed as readJson is. */
export function engineJson<T = unknown>(text: string): T {
  const start = text.search(/\S/);
  if (start < 0) throw new Error("the engine wrote no output");
  if (text[start] !== "{" && text[start] !== "[") return JSON.parse(text);
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < text.length; i += 1) {
    const c = text[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (c === "\\") escaped = true;
      else if (c === '"') inString = false;
    } else if (c === '"') {
      inString = true;
    } else if (c === "{" || c === "[") {
      depth += 1;
    } else if (c === "}" || c === "]") {
      depth -= 1;
      if (depth === 0) return JSON.parse(text.slice(start, i + 1));
    }
  }
  // Unbalanced: let JSON.parse say where it breaks.
  return JSON.parse(text.slice(start));
}
