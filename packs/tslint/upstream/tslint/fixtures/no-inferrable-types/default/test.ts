// errors, inferrable type is declared
let a: number = 7;
let b: boolean = false;
let c: string = "foo";
let d: number = Infinity;
let e: number = -Infinity;
class C {
    x: number = 1;
}

// errors, types are inferrable
function foo (a: number = 5, b: boolean = true, c: string = "bah") { }

class Foo {
    bar: number = 0;
    baz: boolean = true,
    bas: string = "moar";
    readonly foo: boolean = false;
    constructor(readonly foobar: string = "test", public barfoo: number = 1) {}
}

// not errors, inferrable type is not declared
let _x = 7;
let _y = false;
let _z = "foo";

// not error, type is not inferrable
let weird: any = 123;

// not errors, inferrable type is not declared
function bar(a = 5, b = true, c = "bah") { }

// not errors, types are not inferrable
function baz(a: any = 5, b: any = true, c: any = "bah") { }

