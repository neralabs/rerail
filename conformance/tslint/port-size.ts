/** Size of a port: bytes plus the shape of its ast-grep rule trees, summed over the default doc and every override. */
export interface PortSize {
  /** Bytes of the default doc, `packs/tslint/rules/<rule>.json` or `candidates/<rule>.json` (0 when the port has overrides only). */
  defaultBytes: number;
  /** Bytes of the default doc plus every override doc. */
  totalBytes: number;
  docs: number;
  checks: number;
  /** inside + has + precedes + follows. */
  relations: number;
  inside: number;
  has: number;
  precedes: number;
  follows: number;
  not: number;
  any: number;
  all: number;
  /** Deepest rule-object nesting in any clause; a rule with no sub-rules has depth 1. */
  maxDepth: number;
}

export type RuleShape = Omit<PortSize, "defaultBytes" | "totalBytes" | "docs">;

const RELATIONS = ["inside", "has", "precedes", "follows"] as const;

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);

export const emptyShape = (): RuleShape => ({ checks: 0, relations: 0, inside: 0, has: 0, precedes: 0, follows: 0, not: 0, any: 0, all: 0, maxDepth: 0 });

/** Counts one rule object and its sub-rules into `shape`; returns its depth. */
function walkRule(rule: Record<string, unknown>, shape: RuleShape): number {
  let deepest = 0;
  for (const relation of RELATIONS) {
    const sub = rule[relation];
    if (!isObject(sub)) continue;
    shape[relation] += 1;
    shape.relations += 1;
    deepest = Math.max(deepest, walkRule(sub, shape));
  }
  if (isObject(rule.not)) {
    shape.not += 1;
    deepest = Math.max(deepest, walkRule(rule.not, shape));
  }
  for (const key of ["any", "all"] as const) {
    const list = rule[key];
    if (!Array.isArray(list)) continue;
    shape[key] += 1;
    for (const item of list) if (isObject(item)) deepest = Math.max(deepest, walkRule(item, shape));
  }
  return deepest + 1;
}

/** Finds every `rule`, `utils` and `constraints` rule object anywhere in the document (clauses may nest them under forbid/select/exists). */
function walkDocument(value: unknown, shape: RuleShape): void {
  if (Array.isArray(value)) {
    for (const item of value) walkDocument(item, shape);
    return;
  }
  if (!isObject(value)) return;
  for (const [key, child] of Object.entries(value)) {
    if (key === "rule" && isObject(child)) shape.maxDepth = Math.max(shape.maxDepth, walkRule(child, shape));
    else if ((key === "utils" || key === "constraints") && isObject(child)) {
      // An ast query's utility and constraint rules count like its `rule`; a `matches` reference counts nothing.
      for (const sub of Object.values(child)) if (isObject(sub)) shape.maxDepth = Math.max(shape.maxDepth, walkRule(sub, shape));
    } else walkDocument(child, shape);
  }
}

export function documentShape(document: unknown): RuleShape {
  const shape = emptyShape();
  if (isObject(document) && Array.isArray(document.clauses)) shape.checks = document.clauses.length;
  walkDocument(document, shape);
  return shape;
}

/** `texts` are JSON document sources; an unparsable one still counts its bytes. */
export function portSize(defaultText: string | undefined, overrideTexts: string[]): PortSize {
  const texts = defaultText === undefined ? overrideTexts : [defaultText, ...overrideTexts];
  const total = emptyShape();
  for (const text of texts) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch {
      continue;
    }
    const shape = documentShape(parsed);
    for (const key of Object.keys(total) as Array<keyof RuleShape>) total[key] = key === "maxDepth" ? Math.max(total[key], shape[key]) : total[key] + shape[key];
  }
  const bytes = (text: string): number => Buffer.byteLength(text, "utf8");
  return {
    defaultBytes: defaultText === undefined ? 0 : bytes(defaultText),
    totalBytes: texts.reduce((sum, text) => sum + bytes(text), 0),
    docs: texts.length,
    ...total,
  };
}
