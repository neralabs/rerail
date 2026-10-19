
function foo(cb) {
  cb =
    (0,
    function (a) {
      cb(1 + a);
    });
}
foo();
      