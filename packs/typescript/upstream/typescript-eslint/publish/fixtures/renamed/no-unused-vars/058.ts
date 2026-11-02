
import { Nullable } from 'nullable';
import { SomeOther } from 'other';
interface A<V extends Nullable> {
  bar: V;
}
export const a: A<SomeOther> = {
  itx: 'bar',
};
    