let x: Array<undefined> = [undefined] as undefined[];
let y: string[] = <Array<string>>["2"];
let z: Array = [3, "4"];
let u: Array<unknown> = [];

let xx: Array<Array<number>> = [[1, 2], [3]];
let yy: number[][] = [[4, 5], [6]];

let ya = [[1, "2"]] as[number, string][];

type Arr<T> = Array<T>;

// Ignore user defined aliases
let yyyy: Arr<Array<Arr<string>>[]> = [[[["2"]]]];

interface ArrayClass<T> {
    foo: Array<T>;
    bar: T[];
    baz: Arr<T>;
}

function fooFunction(foo: Array<ArrayClass<string>>) {
    return foo.map(e => e.foo);
}

function barFunction(bar: ArrayClass<String>[]) {
    return bar.map(e => e.bar);
}

function bazFunction(baz: Arr<ArrayClass<String>>) {
    return baz.map(e => e.baz);
}

let fooVar: Array<(c: number) => number>;
let barVar: ((c: number) => number)[];

type fooUnion = Array<string|number|boolean>;
type barUnion = (string|number|boolean)[];

type fooIntersection = Array<string & number>;
type barIntersection = (string & number)[];

interface FooInterface {
    '.bar': {baz: string[];};
}

// Test an unknown Array
let t: unknown[] = [];
let u: Array<unknown> = [];

