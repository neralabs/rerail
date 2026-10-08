/* global declaredGlobal, offGlobal: off */
missing();
declaredGlobal();
offGlobal();
typeof maybe;
typeof (maybeParenthesized);
void missingInVoid;
undeclaredWrite = 1;
Math.max(1);
window.alert(1);
const local = 1;
local();
function shadow(Math) { return Math; }
class A { method() { return this.value + inClass; } }
const jsx = { Tag: 1 };
