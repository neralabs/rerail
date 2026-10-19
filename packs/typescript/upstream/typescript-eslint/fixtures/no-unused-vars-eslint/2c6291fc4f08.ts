
function foo(cb) {
  cb = (function () {
    function something(a) {
      cb(1 + a);
    }
    register(something);
  })();
}
foo();
    