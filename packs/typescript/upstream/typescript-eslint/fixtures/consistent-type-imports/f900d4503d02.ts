
import Foo from 'foo';
function fn() {
  type Foo = {}; // shadowing
  let foo: Foo;
}
        