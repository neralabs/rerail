undefined;
function plainTyped(undefined: number) { return undefined; }
function plain(undefined) { return undefined; }
function arrayParam([undefined]) { return undefined; }
function objectParam({ a: undefined }) { return undefined; }
function objectParamTyped({ a: undefined }: { a: number }) { return undefined; }
function defaulted(undefined = 1) { return undefined; }
function rest(...undefined) { return undefined; }
class C { m(undefined) { return undefined; } }
const arrow = (undefined) => undefined;
{ const [undefined] = [1]; undefined; }
{ const { a: undefined } = { a: 1 }; undefined; }
{ const { a: undefined = 2 } = { a: 1 }; undefined; }
{ let undefined = 1; undefined; }
{ try {} catch (undefined) { undefined; } }
for (const undefined of []) { undefined; }
{ const { undefined } = { undefined: 1 }; undefined; }
function after() { return undefined; }
{ function undefined() {} undefined; }
{ class undefined {} undefined; }
namespace N1 { import undefined = M.x; undefined; }
namespace N2 { enum undefined { A } undefined; }
const fnExpr = function undefined() { return undefined; };
const clsExpr = class undefined { m() { return undefined; } };
