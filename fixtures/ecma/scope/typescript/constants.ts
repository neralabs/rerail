// Which bindings carry a value (eslint-utils' canBeConsideredConst) and which values are mutated.
const constant = 1;
let neverWritten = "a";
let written = 2;
written = 3;
let compound = 4;
compound += 1;
let logical = 5;
logical ??= 6;
let incremented = 7;
incremented++;
let decremented = 8;
--decremented;
var redeclared = 9;
var redeclared;
let uninitialized;
uninitialized = 10;
let [destructured] = [11];
const { property } = { property: 12 };
let arrayTarget = 13;
[arrayTarget] = [14];
let objectTarget = 15;
({ objectTarget } = { objectTarget: 16 });
let renamedTarget = 17;
({ key: renamedTarget } = { key: 18 });
let defaultTarget = 19;
[defaultTarget = constant] = [];
let restTarget = 20;
[...restTarget] = [];
let loopTarget = 21;
for (loopTarget of []) {
}
let inTarget = "k";
for (inTarget in {}) {
}
let parenthesized = 22;
(parenthesized) = 23;
let asserted = 24;
(asserted as number) = 25;
let nonNull = 26;
nonNull! = 27;
const pushed: number[] = [];
pushed.push(1);
const sorted = [3, 1];
sorted.sort();
const sliced = [1, 2];
sliced.slice();
const assignedProperty = { a: 1 };
assignedProperty.a = 2;
const deepProperty = { a: { b: 1 } };
deepProperty.a.b = 2;
const indexed = [1];
indexed[0] = 2;
const updatedProperty = { n: 1 };
updatedProperty.n++;
const computedCall = [1];
computedCall["push"](2);
const read = { a: 1 };
read.a;
function declared() {}
class Declared {}
function parameters(parameter: number, withDefault = 1) {
  const inner = parameter + withDefault;
  let closureWritten = 1;
  const closure = () => {
    closureWritten = 2;
  };
  return inner + closureWritten + closure.length;
}
const typeAndValue = 1;
type typeAndValue = number;
interface merged {
  a: number;
}
const merged = 2;

export const exported = 28;

use(
  constant,
  neverWritten,
  written,
  compound,
  logical,
  incremented,
  decremented,
  redeclared,
  uninitialized,
  destructured,
  property,
  arrayTarget,
  objectTarget,
  renamedTarget,
  defaultTarget,
  restTarget,
  loopTarget,
  inTarget,
  parenthesized,
  asserted,
  nonNull,
  pushed,
  sorted,
  sliced,
  assignedProperty,
  deepProperty,
  indexed,
  updatedProperty,
  computedCall,
  read,
  declared,
  Declared,
  parameters,
  typeAndValue,
  merged,
  exported,
);
