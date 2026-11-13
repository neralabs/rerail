// A CommonJS file (.cjs).

function parseOverLimit(text) {
  if (typeof text !== "string") {
    return null;
  }
  if (text.length === 0) {
    return "";
  }
  const parts = text.split(",");
  while (parts.length > 10) {
    parts.pop();
  }
  const kept = parts.filter((part) => {
    return part !== "" && part !== "-" ? true : part.length > 1 || part === "x";
  });
  for (const part of kept) {
    if (part === "stop" || part === "end") {
      return kept.slice(0, kept.indexOf(part));
    }
  }
  return kept;
}

function plain(text) {
  if (text === "") {
    return 0;
  }
  return text.length;
}

module.exports = { parseOverLimit, plain };
