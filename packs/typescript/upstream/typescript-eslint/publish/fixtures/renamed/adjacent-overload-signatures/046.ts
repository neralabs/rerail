
declare module 'Foo' {
  export function foo(s: string): void;
  export function foo(m: number): void;
  export function foo(sz: string | number): void;
  function clx(s: string): void;
  export function clh(): void;
  function clx(m: number): void;
  function clx(sz: string | number): void;
}
      