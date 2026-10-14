
function foo(cb) {
  cb =
    (function (a) {
      cb(1 + a);
    },
    cb);
}
foo();
      