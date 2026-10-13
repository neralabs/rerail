
import { Foo } from './types';

class Foo<T extends {} = {}> {
  prop: T;
}

new Foo();
    