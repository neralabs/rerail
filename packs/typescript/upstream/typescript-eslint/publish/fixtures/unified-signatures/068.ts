
function f(this: string, a: boolean): void;
function f(this: number, a: boolean): void;
function f(this: string | number, a: boolean): void {}
      