
import { Nullable } from 'nullable';
import { SomeOther } from 'other';
function foo<T extends Nullable>(): T {}
foo<SomeOther>();
    