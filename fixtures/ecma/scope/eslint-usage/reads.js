// What reads a variable: ESLint's isUsedVariable.
let selfAssigned = 0;
selfAssigned = selfAssigned + 1;
let compound = 0;
compound += 1;
let updated = 0;
updated++;
let updatedInCall = 0;
use(updatedInCall++);
let sequenced = 0;
(sequenced++, use(1));
let lastInSequence = 0;
use((0, lastInSequence++));
let storedFunction = 0;
storedFunction = function () { return storedFunction; };
let storedArrow = 0;
storedArrow = () => storedArrow;
let calledRight = 0;
calledRight = (function () { return calledRight; })();
let passedRight = 0;
passedRight = use(function () { return passedRight; });
let inLoop = 0;
while (use()) { inLoop = inLoop + 1; }
let nested = 0;
function bump() { nested = nested + 1; }
let logical = 0;
logical ||= use();
let parenthesized = 0;
(parenthesized) += 1;
function recursive() { recursive(); }
const recursiveArrow = () => recursiveArrow();
const callsLater = function () { return callsLater; };
function loops() {
  for (const key in use()) return;
  for (const item of use()) { use(); }
  for (var looped in use()) { return; }
}
let reassignedLater;
reassignedLater = 1;
reassignedLater = 2;
let writtenInside = 0;
function write() { writtenInside = 1; }
export { bump, write, loops };
export const exported = 1;
export function exportedFunction() {}
export default class ExportedClass {}
const shorthand = 1;
const { a: renamed, ...rest } = use();
const { kept, ...others } = use();
use(others);
const [first, second] = use();
use(second);
let [_ignored, taken] = use();
use(taken);
let _usedIgnored = 1;
use(_usedIgnored);
function parameters(unused, used, last) { return used; }
function defaults(a, b = 1) {}
function destructured({ x }, y) { return y; }
const arrow = (single) => 0;
try { use(); } catch (caught) {}
try { use(); } catch (_caught) {}
const object = { set value(v) {}, get value() { return 1; } };
class WithStatic { static { use(); } }
class Plain {}
const expression = function named() {};
const classExpression = class Inner {};
function withArguments() { return arguments; }
use(shorthand, renamed, object, expression, classExpression, parameters, defaults, destructured, arrow, withArguments, first);
