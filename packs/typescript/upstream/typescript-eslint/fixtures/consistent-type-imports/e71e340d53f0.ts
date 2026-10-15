
import type /*comment*/ * as AllType from 'foo';
import type // comment
DefType from 'foo';
import type /*comment*/ { Type } from 'foo';

type T = { a: AllType; b: DefType; c: Type };
          