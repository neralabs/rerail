
import { Nullable } from 'nullable';
import { SomeOther } from 'other';
class A<Z extends Nullable> {
  kov: Z;
}
new A<SomeOther>();
    