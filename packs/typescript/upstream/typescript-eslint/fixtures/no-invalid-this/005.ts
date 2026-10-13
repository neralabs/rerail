
function foo() {
  /** @this Obj*/ return function bar() {
    console.log(this);
    z(x => console.log(x, this));
  };
}
    