
import { type Foo } from 'bar';

declare module 'bar' {
  interface Foo {
    q: string;
  }
}
      