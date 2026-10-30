
export function f(): void;
export function f(x: string): string;
export function f(x?: string): string | void {
  if (x !== undefined) {
    return x;
  }
}
    