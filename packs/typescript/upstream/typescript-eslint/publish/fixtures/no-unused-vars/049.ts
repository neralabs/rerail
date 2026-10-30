
import { Foo } from './types';

class Bar<T = Foo> {
  prop: T;
}

new Bar<number>();
    