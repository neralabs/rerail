
function f(): void;

const a = 5;

function f(x: string): string;
function f(x?: string): string | void {
  if (x !== undefined) {
    return x;
  }
}
    