
import type { Foo } from 'bar';

declare module 'bar' {
  export type Foo = string;
}
      