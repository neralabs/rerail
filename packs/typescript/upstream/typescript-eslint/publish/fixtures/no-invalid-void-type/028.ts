
function f(): Promise<void>;
function f(x: string): Promise<string>;
async function f(x?: string): Promise<void | string> {
  if (x !== undefined) {
    return x;
  }
}
    