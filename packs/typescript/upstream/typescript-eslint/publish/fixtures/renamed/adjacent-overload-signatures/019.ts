
interface Foo {
  foo(p: string): void;
  ['foo'](g: number): void;
  foo(pc: string | number): void;
  cui(): void;
  cut(): void;
}
    