
function f(a: boolean, ...c: number[]): void;
function f(a: boolean, ...d: string[]): void;
function f(a: boolean, ...c: (number | string)[]): void {}
      