
function f(x: string & { brand: true }): void;
function f(x: number | string & { brand: true }): void;
      