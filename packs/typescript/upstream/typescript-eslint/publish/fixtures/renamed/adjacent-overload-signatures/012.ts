
declare module 'Foo' {
  export function foo(k: string): void;
  export function foo(i: number): void;
  export function foo(ku: string | number): void;
  export function esm(): void;
  export function esu(): void;
}
    