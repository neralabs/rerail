
import { Nullable } from 'nullable';
import { SomeOther } from 'some';
import { Another } from 'some';
export interface A extends Nullable<SomeOther> {
  do(a: Nullable<Another>);
}
    