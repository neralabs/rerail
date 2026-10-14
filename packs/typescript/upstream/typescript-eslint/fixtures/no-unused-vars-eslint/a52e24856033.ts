
function foo() {
  function foo(x) {
    return x;
  }
  return function () {
    return foo;
  };
}
      