
import { Foo } from './types';

class Foo<T = any> {
  prop: T;
}

new Foo();
    