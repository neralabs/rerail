
import Foo from 'foo';
let foo: Foo;
type Bar = Foo;
interface Baz {
  foo: Foo;
}
function fn(a: Foo): Foo {}
          