declare const x: boolean;

x === true;
true === x;

x === false;
false === x;

x !== true;
true !== x;

x !== false;
false !== x;

x == true;

declare const y: boolean | undefined;
y === true;

declare function f(): boolean;
f() === false;

declare const a: number, b: number;

a as any as boolean === false;

a < b === false;

!x === false;

