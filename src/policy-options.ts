import type { PolicyDslOptionCondition, PolicyDslOptionDeclaration, PolicyDslValue } from "./executable-policy.js";

/**
 * Document options (a Rerail extension), as the kernel's `compile/config.rs` resolves them: typed
 * values a `define` declares with defaults and a `config` may set, read with `$option` before the
 * document is validated. The two are tested in lockstep; keep them identical.
 */

/** A problem with a document's options or references: where, and what. */
export class PolicyOptionError extends Error {
  constructor(
    readonly path: (string | number)[],
    message: string,
  ) {
    super(message);
    this.name = "PolicyOptionError";
  }
}

type Path = (string | number)[];
type JsonObject = { [key: string]: PolicyDslValue };

const DECLARATION_KEYS = ["type", "items", "enum", "nullable", "default", "description"];
const REFERENCE_KEYS = ["$option", "cases", "default", "escape", "map", "join", "template", "spread"];
const ITEM_TYPES = ["string", "number", "integer", "boolean"];

const isObject = (value: unknown): value is JsonObject => typeof value === "object" && value !== null && !Array.isArray(value);
const fail = (path: Path, message: string): never => {
  throw new PolicyOptionError(path, message);
};

/** Whether a name may name an option: a letter or `_`, then letters, digits, `_`, `-` and `.`. */
export function isPolicyOptionName(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_.-]*$/.test(name);
}

/** JSON equality, as serde_json's `Value` equality (numbers by value). */
function equal(left: unknown, right: unknown): boolean {
  if (Array.isArray(left)) return Array.isArray(right) && left.length === right.length && left.every((item, index) => equal(item, right[index]));
  if (isObject(left)) {
    if (!isObject(right)) return false;
    const keys = Object.keys(left);
    return keys.length === Object.keys(right).length && keys.every(key => Object.hasOwn(right, key) && equal(left[key], right[key]));
  }
  return left === right;
}

function itemMatches(items: string, value: unknown): boolean {
  switch (items) {
    case "string":
      return typeof value === "string";
    case "number":
      return typeof value === "number";
    case "integer":
      return typeof value === "number" && Number.isInteger(value);
    default:
      return typeof value === "boolean";
  }
}

function rejectUnknown(fields: JsonObject, known: readonly string[], path: Path): void {
  for (const key of Object.keys(fields)) {
    if (!known.includes(key)) fail(path, `unknown field \`${key}\``);
  }
}

/**
 * Every entrypoint holds an option number as the double JavaScript parses. An integer beyond the safe range may be a
 * rounded neighbour of the one written, which only raw JSON could tell, so it is refused everywhere (as the native
 * resolver's `canonical_number` does); so is a non-finite number, which JSON cannot write.
 */
function checkNumbers(value: unknown, path: Path): void {
  if (typeof value === "number") {
    if (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value))) {
      fail(path, `${String(value)} is not exactly representable as a JavaScript number`);
    }
  } else if (Array.isArray(value)) value.forEach((item, index) => checkNumbers(item, [...path, index]));
}

/** Checks a value against an option's declaration. */
function checkValue(declaration: PolicyDslOptionDeclaration, value: unknown, path: Path): void {
  checkNumbers(value, path);
  if (value === null) {
    if (declaration.nullable !== true) fail(path, "null is not allowed; the option is not nullable");
    return;
  }
  const allowed = (item: unknown): boolean => declaration.enum === undefined || declaration.enum.some(option => equal(option, item));
  if (declaration.type === "array") {
    if (!Array.isArray(value)) fail(path, "must be an array");
    (value as unknown[]).forEach((item, index) => {
      if (!itemMatches(declaration.items ?? "string", item) || !allowed(item)) fail([...path, index], "is not an allowed item of the option");
    });
    return;
  }
  if (!itemMatches(declaration.type, value)) fail(path, "is not a value of the option's type");
  if (!allowed(value)) fail(path, "is not one of the option's enum values");
}

function readDeclaration(value: unknown, path: Path): PolicyDslOptionDeclaration {
  if (!isObject(value)) fail(path, "must be an object");
  const fields = value as JsonObject;
  rejectUnknown(fields, DECLARATION_KEYS, path);
  const type = fields.type;
  if (typeof type !== "string" || !["boolean", "string", "number", "integer", "array"].includes(type)) {
    fail([...path, "type"], "must equal boolean, string, number, integer or array");
  }
  if (fields.items !== undefined) {
    if (type !== "array") fail([...path, "items"], "only an array option has items");
    if (typeof fields.items !== "string" || !ITEM_TYPES.includes(fields.items)) fail([...path, "items"], "must equal string, number, integer or boolean");
  }
  if (fields.nullable !== undefined && typeof fields.nullable !== "boolean") fail([...path, "nullable"], "must be true or false");
  if (fields.enum !== undefined) {
    if (!Array.isArray(fields.enum) || fields.enum.length === 0) fail([...path, "enum"], "must be a non-empty array");
    checkNumbers(fields.enum, [...path, "enum"]);
    (fields.enum as unknown[]).forEach((option, index) => {
      if (!itemMatches(type === "array" ? String(fields.items ?? "string") : String(type), option)) {
        fail([...path, "enum", index], "is not a value of the option's type");
      }
    });
  }
  if (!Object.hasOwn(fields, "default")) fail([...path, "default"], "is required");
  const declaration = fields as unknown as PolicyDslOptionDeclaration;
  checkValue(declaration, fields.default, [...path, "default"]);
  return declaration;
}

/** The key a value selects in `cases`: a string's text, anything else's JSON text. */
function caseKey(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value);
}

function text(value: unknown, path: Path): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return JSON.stringify(value);
  return fail(path, "the option's value is not text (a string, a number or a boolean)");
}

/** Rust's `regex::escape`: a backslash before every meta character. */
function escapeRegex(value: string): string {
  return value.replace(/[\\.+*?()|[\]{}^$#&\-~]/g, character => `\\${character}`);
}

type Resolved = { value: PolicyDslValue } | { spread: PolicyDslValue[] };

class Options {
  constructor(readonly values: Map<string, PolicyDslValue>) {}

  get(name: string, path: Path): PolicyDslValue {
    if (!this.values.has(name)) fail(path, `\`${name}\` is not an option this document declares`);
    return this.values.get(name)!;
  }

  substitute(value: PolicyDslValue, path: Path): PolicyDslValue {
    if (isObject(value) && Object.hasOwn(value, "$option") && value.spread === true) fail([...path, "spread"], "only an array's item may spread");
    const resolved = this.reference(value, path);
    if (resolved !== undefined) {
      if ("spread" in resolved) return fail([...path, "spread"], "only an array's item may spread");
      return resolved.value;
    }
    return this.substituteInner(value, path);
  }

  substituteInner(value: PolicyDslValue, path: Path): PolicyDslValue {
    if (Array.isArray(value)) {
      const out: PolicyDslValue[] = [];
      value.forEach((item, index) => {
        const resolved = this.reference(item, [...path, index]);
        if (resolved === undefined) out.push(this.substituteInner(item, [...path, index]));
        else if ("spread" in resolved) out.push(...resolved.spread);
        else out.push(resolved.value);
      });
      return out;
    }
    if (isObject(value)) {
      return Object.fromEntries(Object.entries(value).map(([key, child]) => [key, this.substitute(child, [...path, key])]));
    }
    return value;
  }

  private resolveCases(value: JsonObject, option: PolicyDslValue, path: Path): PolicyDslValue {
    if (!isObject(value.cases)) return fail([...path, "cases"], "must be an object");
    for (const key of ["escape", "map", "join", "template"]) {
      if (value[key] !== undefined) fail([...path, key], `cases selects a value; it has no ${key}`);
    }
    const key = caseKey(option);
    const cases = value.cases as JsonObject;
    if (Object.hasOwn(cases, key)) return this.substitute(cases[key]!, [...path, "cases", key]);
    if (Object.hasOwn(value, "default")) return this.substitute(value.default!, [...path, "default"]);
    return fail([...path, "cases"], `no case for \`${key}\` and no default`);
  }

  reference(value: PolicyDslValue, path: Path): Resolved | undefined {
    if (!isObject(value) || !Object.hasOwn(value, "$option")) return undefined;
    rejectUnknown(value, REFERENCE_KEYS, path);
    const name = value.$option;
    if (typeof name !== "string") fail([...path, "$option"], "must name an option");
    const option = this.get(name as string, [...path, "$option"]);
    if (value.spread !== undefined && typeof value.spread !== "boolean") fail([...path, "spread"], "must be true or false");
    const resolved = value.cases !== undefined
      ? this.resolveCases(value, option, path)
      : this.transformReference(value, option, path);
    if (value.spread !== true) return { value: resolved };
    if (!Array.isArray(resolved)) return fail([...path, "spread"], "the value spread must be an array");
    return { spread: resolved };
  }

  private transformReference(value: JsonObject, option: PolicyDslValue, path: Path): PolicyDslValue {
    if (Object.hasOwn(value, "default")) fail([...path, "default"], "only cases have a default");
    return this.transform(option, value, path);
  }

  transform(option: PolicyDslValue, fields: JsonObject, path: Path): PolicyDslValue {
    const field = (key: string): string | undefined => {
      if (fields[key] === undefined) return undefined;
      if (typeof fields[key] !== "string") fail([...path, key], "must be a string");
      return fields[key] as string;
    };
    let value = option;
    const escape = field("escape");
    if (escape !== undefined) {
      if (escape !== "regex") fail([...path, "escape"], `unknown escape \`${escape}\`; expected regex`);
      value = Array.isArray(value) ? value.map(item => escapeRegex(text(item, path))) : escapeRegex(text(value, path));
    }
    const map = field("map");
    if (map !== undefined) {
      if (!Array.isArray(value)) fail([...path, "map"], "the option's value is not an array");
      value = (value as PolicyDslValue[]).map(item => {
        const itemText = text(item, path);
        return map.replaceAll("{}", () => itemText);
      });
    }
    const join = field("join");
    if (join !== undefined) {
      if (!Array.isArray(value)) fail([...path, "join"], "the option's value is not an array");
      value = (value as PolicyDslValue[]).map(item => text(item, path)).join(join);
    }
    const template = field("template");
    if (template !== undefined) {
      const valueText = text(value, path);
      value = template.replaceAll("{}", () => valueText);
    }
    return value;
  }
}

/** Whether an option's value meets a `when.option` condition. */
export function policyOptionConditionHolds(value: PolicyDslValue, condition: PolicyDslOptionCondition, path: Path = []): boolean {
  if (!isObject(condition)) return equal(value, condition);
  const entries = Object.entries(condition);
  if (entries.length !== 1) return fail(path, "must be a value or one of { in }, { includes }, { not }");
  const [key, operand] = entries[0]!;
  switch (key) {
    case "in":
      if (!Array.isArray(operand)) fail([...path, "in"], "must be an array");
      return (operand as PolicyDslValue[]).some(option => equal(option, value));
    case "includes":
      if (!Array.isArray(value)) fail([...path, "includes"], "the option is not an array");
      return (value as PolicyDslValue[]).some(item => equal(item, operand));
    case "not":
      return !policyOptionConditionHolds(value, operand as PolicyDslOptionCondition, [...path, "not"]);
    default:
      return fail(path, `unknown condition \`${key}\`; expected in, includes or not`);
  }
}

function policyDefines(document: unknown): Array<[Path, JsonObject]> {
  const checks = isObject(document) && Array.isArray(document.clauses) ? document.clauses : [];
  return checks.flatMap((check, index) =>
    isObject(check) && !Object.hasOwn(check, "$option") && isObject(check.define) ? [[["clauses", index, "define"], check.define] as [Path, JsonObject]] : [],
  );
}

function optionDeclarations(defines: Array<[Path, JsonObject]>): Map<string, PolicyDslOptionDeclaration> {
  const declared = new Map<string, PolicyDslOptionDeclaration>();
  for (const [path, define] of defines) {
    if (define.options === undefined) continue;
    if (!isObject(define.options)) fail([...path, "options"], "must be an object");
    for (const [name, value] of Object.entries(define.options as JsonObject)) {
      const optionPath = [...path, "options", name];
      if (!isPolicyOptionName(name)) fail(optionPath, "an option's name starts with a letter or _ and holds letters, digits, _, - and .");
      if (declared.has(name)) fail(optionPath, `option \`${name}\` is already declared by another define`);
      declared.set(name, readDeclaration(value, optionPath));
    }
  }
  return declared;
}

/** The effective values of a document's options: each `config` value, else its default. */
export function policyOptionValues(document: unknown): Map<string, PolicyDslValue> {
  const defines = policyDefines(document);
  const declared = optionDeclarations(defines);
  const values = new Map([...declared].map(([name, declaration]) => [name, declaration.default]));
  for (const [path, define] of defines) {
    if (define.config === undefined) continue;
    if (!isObject(define.config)) fail([...path, "config"], "must be an object");
    for (const [name, value] of Object.entries(define.config as JsonObject)) {
      const valuePath = [...path, "config", name];
      const declaration = declared.get(name);
      if (declaration === undefined) fail(valuePath, `\`${name}\` is not an option this document declares`);
      checkValue(declaration!, value, valuePath);
      values.set(name, value);
    }
  }
  return values;
}

/**
 * The document with every `$option` reference replaced by what it stands for (`compile/config.rs`
 * `resolve`). A define's `options` and `config` are read as written, and so are `format`, `id` and
 * `revision`. A reference among the clauses stands for a clause or, spread, for several; never for a
 * define. Throws a `PolicyOptionError` where the options or a reference are invalid.
 */
export function resolvePolicyOptions(document: unknown): unknown {
  const options = new Options(policyOptionValues(document));
  if (!isObject(document)) return document;
  const resolved: JsonObject = {};
  for (const [key, value] of Object.entries(document)) {
    if (key === "format" || key === "id" || key === "revision") resolved[key] = value;
    else if (key === "clauses" && Array.isArray(value)) {
      resolved[key] = value.flatMap((check, index) => {
        const path = ["clauses", index];
        const reference = options.reference(check, path);
        if (reference !== undefined) {
          const items = "spread" in reference ? reference.spread : [reference.value];
          if (items.some(item => isObject(item) && Object.hasOwn(item, "define"))) {
            fail(path, "a define is written among the clauses, not made by a $option reference");
          }
          return items;
        }
        if (!isObject(check) || !isObject(check.define)) return [options.substitute(check, path)];
        return [Object.fromEntries(
          Object.entries(check).map(([field, child]) => [
            field,
            field === "define"
              ? Object.fromEntries(
                  Object.entries(child as JsonObject).map(([part, body]) => [
                    part,
                    part === "options" || part === "config" ? body : options.substitute(body, [...path, "define", part]),
                  ]),
                )
              : options.substitute(child, [...path, field]),
          ]),
        )];
      });
    } else resolved[key] = options.substitute(value, [key]);
  }
  return resolved;
}

/**
 * A variant as a patch: the document with `patch` (option name to value) merged into the `config`
 * of the define declaring each option, checked against its declaration. The catalog stores the
 * result resolved, so a variant is a configuration of one document, not a copy of it.
 */
export function applyPolicyConfig<T>(document: T, patch: Record<string, PolicyDslValue>): T {
  const copy = structuredClone(document) as unknown;
  const checks = isObject(copy) && Array.isArray(copy.clauses) ? copy.clauses : [];
  for (const [name, value] of Object.entries(patch)) {
    const owner = checks.find(check => isObject(check) && !Object.hasOwn(check, "$option") && isObject(check.define) && isObject(check.define.options) && Object.hasOwn(check.define.options, name));
    if (owner === undefined) fail(["config", name], `\`${name}\` is not an option this document declares`);
    const define = (owner as JsonObject).define as JsonObject;
    define.config = { ...(isObject(define.config) ? define.config : {}), [name]: value };
  }
  policyOptionValues(copy);
  return copy as T;
}
