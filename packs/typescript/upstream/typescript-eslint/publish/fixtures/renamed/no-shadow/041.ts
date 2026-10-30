
import type { Foo } from 'bar';

declare module 'baz' {
  export interface Foo {
    k: string;
  }
}
      