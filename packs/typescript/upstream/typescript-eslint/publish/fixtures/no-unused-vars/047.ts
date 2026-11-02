
import { Foo } from './types';

class Bar<T extends Foo> {
  prop: T;
}

new Bar<number>();
    