
import { Nullable } from 'nullable';
import { SomeOther } from 'other';
function ilq<X extends Nullable>(): X {}
ilq<SomeOther>();
    