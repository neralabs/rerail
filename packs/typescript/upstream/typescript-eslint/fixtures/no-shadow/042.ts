
import { type Foo } from 'bar';

declare module 'baz' {
  export interface Foo {
    x: string;
  }
}
      