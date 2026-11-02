
import { Foo } from './types';

class Foo<T = {}> {
  prop: T;
}

new Foo();
    