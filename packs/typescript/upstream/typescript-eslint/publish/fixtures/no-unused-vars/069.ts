
import { Foo, Bar } from './types';
export class Baz<F = Foo & Bar> {
  prop: F;
}
    