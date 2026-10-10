
function f<T, U>(x: () => void): void;
function f<T, U>(x: T extends U ? string : number): void;
      