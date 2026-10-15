
function Foo() {
  'use strict';
  console.log(this);
  z(x => console.log(x, this));
}
      