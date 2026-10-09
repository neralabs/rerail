// Works in non-declaration file too.
function f(x: number): void;
function f(x: string): void;
function f(x: any): any {
    return x;
}

// Body does *not* count as a signature.
function g(): void;
function g(a: number, b: number): void;
function g(a?: number, b?: number): void {}

class C {
    constructor();
    constructor(a: number, b: number);
    constructor(a?: number, b?: number) {}

    a(): void;
    a(a: number, b: number): void;
    a(a?: number, b?: number): void {}
}

function rest(...xs: number[]): void;
function rest(xs: number[], y: string): void;
function rest(...args: any[]) {}

function opt(xs?: number[]): void;
function opt(xs: number[], y: string): void;
function opt(...args: any[]) {}
