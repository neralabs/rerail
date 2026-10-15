
import type Already1Def from 'foo';
import type { Already1 } from 'foo';
import A, { B } from 'foo';
import { C, D, E } from 'bar';
import type { Already2 } from 'bar';
type T = { b: B; c: C; d: D };
          