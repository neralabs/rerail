
import { foo } from 'foo';

export type Foo = typeof foo;

export const bar = (): Foo => foo;
    