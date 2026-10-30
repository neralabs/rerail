
interface Foo {}

declare module 'bar' {
  export interface Foo {
    x: string;
  }
}
      