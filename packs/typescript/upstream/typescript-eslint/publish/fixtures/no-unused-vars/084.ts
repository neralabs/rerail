
export function foo(a: number): number;
export function foo(a: string): string;
export function foo(a: number | string): number | string {
  return a;
}
    