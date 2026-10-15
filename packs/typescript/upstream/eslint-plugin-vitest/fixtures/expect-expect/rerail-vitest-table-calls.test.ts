import { it } from 'vitest';

it.for([1])('for %s', () => {});
it.only.each`a`('tagged', () => {});
