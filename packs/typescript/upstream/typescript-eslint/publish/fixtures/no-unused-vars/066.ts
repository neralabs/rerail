
import { Foo } from './types';
export class Bar<T extends Foo> {
  prop: T;
}
    