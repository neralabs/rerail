// typescript-eslint's additions to what reads a variable.
import { onlyTyped, type TypeOnly } from "m";
import { value } from "m";
import * as namespaceImport from "m";
import type { Unused } from "m";
type Query = typeof onlyTyped;
let annotated: TypeOnly;
const valueQueried = 1;
type ValueQuery = typeof valueQueried;
export type Exported = Query | ValueQuery;
interface Recursive { next: Recursive }
type Alias = { self: Alias };
enum Direction { Up, Down = Up }
namespace Space { export const inside = Space; }
namespace Qualified.Name { export const x = 1; }
function predicate(input: unknown): input is string { return true; }
type Guarded = { kind: 1 };
export function guard(value: unknown): value is Guarded { return true; }
function asserts(input: unknown): asserts input { }
class Properties { constructor(private held: number, readonly seen: number, plain: number) {} }
declare function overloaded(a: number): void;
type Signature = (parameter: string) => void;
interface Methods { method(parameter: number): void }
type Mapped = { [Key in "a" | "b"]: 1 };
type Inferred<T> = T extends (arg: infer A) => infer R ? [A, R] : never;
type Unused2<T> = T;
declare module "ambient" {
  interface AmbientInterface {}
  const ambientConst: number;
  namespace AmbientSpace { const deep: number; }
}
declare global { interface Window { ambientGlobal: number } }
const decorate = (): ParameterDecorator => () => {};
export class Decorated { method(@decorate() decorate: string) {} }
export const used = [value, namespaceImport, annotated, predicate, asserts, Properties, overloaded];
type Merged = 1;
export = Merged;
