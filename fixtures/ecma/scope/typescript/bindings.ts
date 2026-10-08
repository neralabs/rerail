// Name-resolution oracle fixture: every reference to an `undefined*` or out-of-scope name.
import Default, { named as renamed, type TypeOnly } from "module";
import * as namespaceImport from "other";
import required = require("required");
export { renamed as exported, undefinedExport };
export { reExported } from "module";

undefinedCall(Default, renamed, namespaceImport, required);
hoisted();
function hoisted() {
  return arguments;
}

var topVar = 1;
let topLet = topVar;
const { a, b: [c, ...d], e = undefinedDefault } = undefinedSource;
a; c; d; e; b;

function params(p1: number, { p2 }: { p2: number }, p3 = p1, ...rest: number[]) {
  if (p1) {
    var hoistedVar = 1;
    let blockLet = 2;
    function blockFunction() {}
  }
  return [p2, p3, rest, hoistedVar, blockLet, blockFunction];
}

const arrow = (x: number) => x + outerArrow;
const single = y => y;
const named = function self() { return self; };
self;
const Klass = class Inner<T> extends Base<T> implements Iface { field = Inner; method(): T { return undefinedInMethod; } };
Inner;

class Declared<U> {
  static { var staticVar = 1; }
  constructor(private readonly prop: U) {}
  method(): U { return this.prop; }
}
Declared; U; staticVar;

enum Color { Red, Green = Red, Blue = Undefined }
namespace Space { export namespace Inner { export const inside = 1; } export type T = number; }
Space; inside;
declare global { interface Window { extra: string } var augmented: number; }
declare module "ambient" { const ambientConst: number; }
declare const declaredConst: number;
augmented; ambientConst; declaredConst;

type Alias<V> = V extends Array<infer Item> ? Item : Missing;
let aliased: Alias<number>; let item: Item; let v: V;
interface Iface2<W> { call(arg: W): Return; (sig: number): void; new (n: number): Iface2<W>; [key: string]: unknown; }
let mapped: { [K in keyof Iface2<number>]: K };
let k: K; arg; sig; key; n;
type Fn = (fnParam: number) => fnParam;
let typed: typeof topLet; let q: Space.T; let qq: MissingSpace.T;

try { undefinedThrow(); } catch ({ message }) { message; } finally { message; }
for (const key2 in topLet) key2;
for (let i = 0; i < 1; i++) i;
for (loose in obj) {}
label: for (;;) { break label; }
[assignA, assignB] = undefinedPair;
({ assignC } = undefinedObject);
const shorthand = { topLet, missingShorthand, [computedKey]: 1, plain: 2 };
switch (topLet) { case 1: let inSwitch = 1; inSwitch; }
inSwitch;
let u = undefined;
@decorator class Decorated { @member prop = 1; }
const decorate = (): ParameterDecorator => () => {};
class WithParameterDecorators { method(@decorate() decorate: string, @missingDecorator() other: string) { return decorate + other; } }
type ExportedType = 1;
export = ExportedType;
export default missingDefault;
