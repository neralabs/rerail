
type A = 1 | 2;
type B = 3 | 4;
function f<T extends A>(x: T, y: string): void;
function f<T extends B>(x: T): void;
    