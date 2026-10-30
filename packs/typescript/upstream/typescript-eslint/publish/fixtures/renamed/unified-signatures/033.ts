
function f(this: void, b: boolean): void;
function f(this: {}, b: boolean): void;
function f(this: void | {}, b: boolean): void {}
    