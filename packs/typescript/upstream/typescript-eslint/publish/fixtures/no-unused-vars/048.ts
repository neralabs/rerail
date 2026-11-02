
import { Foo, Bar } from './types';

class Baz<T extends Foo & Bar> {
  prop: T;
}

new Baz<any>();
    