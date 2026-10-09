// good

interface i1 {
    a();
    a(x: number);
    b();
    b(x: string);
}

interface i2 {
    a();
    a(x: number);
    a(): void;
    b();
    b(x: string);
}

interface i3 {
    a();
    "a"();
}

interface i4 {
    a();
    ["a"]();
}

interface i5 {
    a(): string;
    bar: {
        a(): number;
    }
}

interface i6 {
    // ensure no false positives for properties/methods available from prototype chain
    toString(): string;
}

interface i7 {
    // Computed properties with different source code text OK
    [Symbol.iterator](): void;
    [Symbol.toStringTag](): void;
}

interface i8 {
    // Computed property with same source code text as regular property OK
    [Symbol.iterator](): void;
    x: number;
    "Symbol.iterator"(): void;
}


// bad

interface b1 {
    a();
    a(x: number);
    b();
    b(x: string);
    a(x: string);
}

interface b2 {
    a();
    a(x: number);
    b();
    b(x: string);
    a(): void;
}

interface b3 {
    a();
    12();
    "a"();
    12();
}

interface b4 {
    a();
    b(): void;
    ["a"](v: number): void;
}

interface b5 {
    a(): string;
    bar: {
        a(): number;
        b();
        a(b: number): void;
    }
}

interface b6 {
    (): void;
    x: number;
    (x: number): number;
}

// Also works in classes, source files, modules, namespaces

class C {
    a(): void;
    b(): void;
    a(x: number): void;
}

declare function a(): void;
declare function b(): void;
declare function a(x: number): void;

declare module "m" {
    export function a(): void;
    export function b(): void;
    export function a(x: number): void;
}

declare namespace N {
    export function a(): void;
    export function b(): void;
    export function a(x: number): void;
}

class Foo {
  public static bar() {}
  private constructor() {}
  public bar() {}
  public constructor(foo: any) {}
}

// A semicolon on its own is a SemicolonClassElement, but still consider them adjacent.
class Bar {
    get test() { return 0; };
    set test(v: number) {};
}

interface I {
    // Catches computed properties with same source code text
    [Symbol.iterator](): void;
    x: number;
    [Symbol.iterator](): void;
}

class Accessors {
    private x: number;
    private y: number;
    get x() {return this.x;}
    get y() {return this.y;}
    set x(newX: number) {this.x = newX;}
    set y(newY: number) {this.y = newY;}
}
