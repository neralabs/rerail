import importEquals = require("./commonjsModule");
import importAlias = importEquals;
namespace N { export const x: any = 0; }
import importQualifiedName = N.x;
import * as namespaceImport from "./es6Module";
import defaultExport, { namedExport, NS } from "./es6Module";
import namedExportAlias = namespaceImport.namedExport;

const num: namespaceImport.T = 0;

{
    const { x: { y, z } } = { x: { y: 1, z: 2 } };
    function f(a: any, b: { x: any }) {
        const { g } = a;
        const { x } = b;
    }
}

// Ignore number/string literal in type
function returnsA(): 0 | "a" | null | undefined {
    return null;
}

label: for (const x of [1]) {
    continue label;
}
label: while(true) {
    break label;
}

importAlias.property;

namespaceImport.namedExport;

namedExport.foo;
namedExportAlias.foo;
namedExport, namedExportAlias;


declare const x: any;
declare const hasProp: { x: any };

hasProp.x(1);

declare function takesAny(a: any, ...bs: any[]): void;
declare function takesNumber(a: number, ...bs: number[]): void;

takesAny(x, x);
takesNumber(x, x);

declare function templateTakesAny(arr: TemplateStringsArray, a: any, ...bs: any[]): any;
declare function templateTakesNumber(arr: TemplateStringsArray, a: number, ...bs: number[]): any;

templateTakesAny`${x}${x}`;
templateTakesNumber`${x}${x}`;
templateTakesAny`${x}`.prop;

declare function decoratorTakesAny(value: any): Function;
declare function decoratorTakesNumber(value: number): Function;
declare const decoratorIsAny: any;

class C {
    @decoratorTakesAny(x) f() {}
    @decoratorTakesNumber(x) g() {}
    @decoratorIsAny h() {}
}

x instanceof Date;
Date instanceof x;

const retBool: () => boolean = x;

function params(a: any = x, b: boolean = x) {}

function f(x: any, retAny: () => any): any {
    x;
    (x);

    x.foo;
    x(0);
    x``;
    x + 3;
    new x();

    retAny();
    retAny()[0];

    // Same for constructors
    new X(x);
    new Y(x);

    // Assignment: assign to 'any' OK, assign to other not OK.
    const v0: any = x;
    const v1: boolean = x;
    let v2: any, v3: boolean;
    v2 = x;
    v3 = x;

    // Return OK if return type is 'any'
    return x;
}

function f2(x: any): boolean {
    return x;
}

class X {
    constructor(y: any) {}

    prop0: any = x;
    prop1: number = x;

}
class Y {
    constructor(x: number) {}
}

function g(x: any): string {
    if (x === undefined) {
        return "undefined";
    }
    if (typeof x === "string") {
        return `string: ${x}`;
    }
    if (x instanceof Array) {
        // Once it's been tested, OK to use it as an array.
        return `Array, length ${x.length}`;
    }
    if (Math.random() > 0.5) {
        // Allow explicit cast to 'any'
        (x as any).whatchamacallit;
        // Allow explicit cast
        return (<string> x).toLowerCase() + (x as string).toUpperCase();
    }

    "" + x;
    x + "";
    let s = "";
    s += x;
    x += s;

    return `other: ${x}`;
}

function deleteOk(obj: { x: any }) {
    delete obj.x;
}

const obj: { x: number, y: number } = {
    x: "number" as any, // Cast OK, and doesn't warn for LHS either.
    [x]: 1,
};

switch (x) {}
switch (x.y) {
    case x:
        x.y;
        break;
    case x.y:
        break;
}

declare global {}

if (x) {}
if (!x) {}
if (!x || x) {}
for (;x;) {}
while (x) {}
do {} while (x);

takesNumber(x ? x : x);
typeof x.prop;

function *gen1(): any {
    yield x;
    (yield).foo;
}

function *gen2() {
    yield x;
}

function *gen3(): IterableIterator<number> {
    yield x;
}

function *gen4(): IterableIterator<any> {
    yield x;
}

void x;

{
    let { a = x, b = x, c: d = x }: { a: number, b: any, c: string } = { a: 1, b: true, c: '' };
}
{
    let {a, b} = x;
    let {c, d}: any = x;
    let {x: {e, f}} = {x};
    let {x: {g, h}}: {x: any} = {x};
    ({a, b} = x);
    ({x: {a, b}} = {x});
}
{
    let obj = {x};
    let {x: {a, b}} = obj;
    ({x: {a, b}} = obj); // TODO show error on the inner destructuring
}
{
    let { a = x, b = x, c: d = x } = { a: 1, b: true, c: '' };
}
{
    let obj: {a: any, b: any} = null!;
    let {a = x, b = x}: {a: number, b: any} = obj;
}
{
    let a: number;
    let b: any;
    let c: string;
    ({a = x, b: b = x, d: c = x} = {a: 1, b: x, d: ''});
}
{
    class C {
        prop: number = x;
        prop2 = x;
        prop3: any = x;
    }
    class D extends C {
        prop = x;
        prop2 = x;
        prop3 = x;
    }
}
{
    let obj = {x};
    let arr = [x];
}
for (const {prop} of [x]) {}

function predicate(x: any): x is number {
    return typeof x === 'number';
}

function hasThisParameter(this: any) {
    const v: number = this;
}

(async function() {
    return x;
});
(async function(): Promise<any> {
    return x;
});
(async function(): Promise<number> {
    return x;
})

{
    let obj = {...x};
    ({...obj} = x);
    let arr = [...x];
    predicate(...x);
}

namespace TestNS {
    export interface ITest {}
}
namespace TestNS2 {
    "";
    export interface ITest {}
}
{
    class Test implements TestNS.ITest {}
    type T = any;
    class Test2 implements T {}
    let C = null as any;
    class Test3 extends C {}

    interface I {}
    class Test4 implements I {}
    class Test5 implements TestNS2.ITest {}
    class Test6 implements NS.ITest {}
}

