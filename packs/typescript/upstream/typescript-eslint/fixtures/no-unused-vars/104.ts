
import { Foo } from './bar';
export type F<A extends unknown[]> = (...a: Foo<A>) => unknown;
    