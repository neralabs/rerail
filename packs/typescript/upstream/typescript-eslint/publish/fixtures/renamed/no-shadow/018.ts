
import type { Foo } from 'bar';

declare module 'bar' {
  export interface Foo {
    p: string;
  }
}
    