
import { Nullable } from 'nullable';
import { SomeOther } from 'some';
class A extends Nullable<SomeOther> {
  other: Nullable<Another>;
}
new A();
    