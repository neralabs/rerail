
type T = string;
function foo<T extends (arg: any) => void>(arg: T) {}
      