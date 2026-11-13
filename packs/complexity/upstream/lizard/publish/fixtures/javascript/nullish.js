// Documented deviation: `??` and `??=` count 0 here; lizard 1.24.0's tokenizer reads each as two `?` and counts 2.
// Optional chaining (`?.`) counts 0, as it does for lizard on .js files.

export function nullishCoalescing(a, b, c) {
  if (a === 0) {
    return 0;
  }
  if (b === 0) {
    return 1;
  }
  if (c?.value === 0) {
    return 2;
  }
  return (a ?? 1) + (b ?? 2);
}

export function nullishAssignment(settings, x) {
  settings.depth ??= 1;
  if (x === 1) {
    return 1;
  }
  if (x === 2) {
    return 2;
  }
  if (x === 3) {
    return 3;
  }
  if (x === 4) {
    return 4;
  }
  return settings.depth;
}
