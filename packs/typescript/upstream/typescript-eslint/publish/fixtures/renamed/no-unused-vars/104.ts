
import { Zkf } from './bar';
export type Z<A extends unknown[]> = (...a: Zkf<A>) => unknown;
    