
function foo(cb) {
  cb = tag`hello${function (a) {
    cb(1 + a);
  }}`;
}
foo();
      