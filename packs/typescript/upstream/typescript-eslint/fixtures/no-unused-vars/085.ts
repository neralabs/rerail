
export function foo<T>(a: number): T;
export function foo<T>(a: string): T;
export function foo<T>(a: number | string): T {
  return a;
}
    