
function f(): void;
function f(x: string): string | void;
function f(x?: string): string | void {
  if (x !== undefined) {
    return x;
  }
}
      