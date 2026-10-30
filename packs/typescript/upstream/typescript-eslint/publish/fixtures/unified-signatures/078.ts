
function f(x: string & { brand: true }): void;
function f(x: number | string & /* brand */ { brand: true }): void;
      