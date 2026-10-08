// Statement and expression decisions: if/else if, loops, catch, switch cases, &&, ||, ?: and ||= / &&=.

export function ifElseAtLimit(x: number): string {
  if (x === 1) {
    return "one";
  } else if (x === 2) {
    return "two";
  } else if (x === 3) {
    return "three";
  } else if (x === 4) {
    return "four";
  } else if (x === 5) {
    return "five";
  } else {
    return "many";
  }
}

export function ifElseOverLimit(x: number): string {
  if (x === 1) {
    return "one";
  } else if (x === 2) {
    return "two";
  } else if (x === 3) {
    return "three";
  } else if (x === 4) {
    return "four";
  } else if (x === 5) {
    return "five";
  } else if (x === 6) {
    return "six";
  }
  return "many";
}

export function loopsAtLimit(rows: number[][], keys: Record<string, number>): number {
  let total = 0;
  for (let index = 0; index < rows.length; index++) {
    total += index;
  }
  for (const key in keys) {
    total += key.length;
  }
  for (const row of rows) {
    total += row.length;
  }
  while (total > 100) {
    total -= 1;
  }
  do {
    total += 1;
  } while (total < 0);
  return total;
}

export function loopsOverLimit(rows: number[][], keys: Record<string, number>): number {
  let total = 0;
  for (let index = 0; index < rows.length; index++) {
    total += index;
  }
  for (const key in keys) {
    total += key.length;
  }
  for (const row of rows) {
    for (const cell of row) {
      total += cell;
    }
  }
  while (total > 100) {
    total -= 1;
  }
  do {
    total += 1;
  } while (total < 0);
  return total;
}

export function catchOverLimit(texts: string[]): number {
  let total = 0;
  try {
    total += 1;
  } catch (error) {
    total -= 1;
  }
  try {
    total += 2;
  } catch {
    total -= 2;
  } finally {
    total += 0;
  }
  for (const text of texts) {
    try {
      total += JSON.parse(text) as number;
    } catch (error) {
      if (error instanceof RangeError) {
        total = 0;
      } else if (error instanceof TypeError) {
        total = -1;
      }
    }
  }
  return total;
}

export function switchAtLimit(command: string): number {
  switch (command) {
    case "start":
      return 1;
    case "stop":
      return 2;
    case "pause":
      return 3;
    case "resume":
      return 4;
    case "reset":
      return 5;
    default:
      return 0;
  }
}

export function switchOverLimit(command: string): number {
  switch (command) {
    case "start":
    case "begin":
      return 1;
    case "stop":
      return 2;
    case "pause":
      return 3;
    case "resume":
      return 4;
    case "reset":
      return 5;
    default:
      return 0;
  }
}

export function logicalAtLimit(a: boolean, b: boolean, c: boolean, d: boolean, e: boolean, f: boolean): boolean {
  return (a && b) || (c && d) || (e && f);
}

export function logicalOverLimit(a: boolean, b: boolean, c: boolean, d: boolean, e: boolean, f: boolean, g: boolean): boolean {
  return (a && b) || (c && d) || (e && f && g);
}

export function ternaryOverLimit(a: boolean, b: boolean, c: boolean, d: boolean, e: boolean, f: boolean): number {
  const first = a ? 1 : 0;
  const second = b ? 1 : 0;
  const third = c ? 1 : 0;
  const fourth = d ? 1 : 0;
  const fifth = e ? 1 : 0;
  const sixth = f ? 1 : 0;
  return first + second + third + fourth + fifth + sixth;
}

export function logicalAssignmentOverLimit(flags: { a: boolean; b: boolean; c: boolean }, x: number): number {
  flags.a ||= x > 1;
  flags.b &&= x > 2;
  flags.c ||= x > 3;
  if (flags.a) {
    return 1;
  }
  if (flags.b) {
    return 2;
  }
  if (flags.c) {
    return 3;
  }
  return 0;
}
