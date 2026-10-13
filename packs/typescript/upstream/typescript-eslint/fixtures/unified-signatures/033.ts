
function f(this: void, a: boolean): void;
function f(this: {}, a: boolean): void;
function f(this: void | {}, a: boolean): void {}
    