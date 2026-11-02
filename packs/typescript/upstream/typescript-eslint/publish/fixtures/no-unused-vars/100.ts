
export function foo<T>(value: T): T {
  return { value };
}
export type Foo<T> = typeof foo<T>;
    