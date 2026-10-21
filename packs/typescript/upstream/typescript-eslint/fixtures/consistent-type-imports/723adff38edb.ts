
import { Value1, Type1 } from 'named_import';
import Type2, { Value2 } from 'default_import';
import Value3, { Type3 } from 'default_import2';
import Type4, { Type5, Value4 } from 'default_and_named_import';
type T = Type1 | Type2 | Type3 | Type4 | Type5;
          