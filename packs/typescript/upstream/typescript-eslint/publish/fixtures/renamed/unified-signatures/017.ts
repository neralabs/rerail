
type I = 1 | 2;
type J = 3 | 4;
function l<P extends I>(w: P, z: string): void;
function l<P extends J>(w: P): void;
    