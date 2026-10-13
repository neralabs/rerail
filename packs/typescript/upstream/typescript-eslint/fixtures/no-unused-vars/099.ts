
type Foo = {
  error: Error | null;
};

export function foo() {
  return new Promise<Foo>();
}
    