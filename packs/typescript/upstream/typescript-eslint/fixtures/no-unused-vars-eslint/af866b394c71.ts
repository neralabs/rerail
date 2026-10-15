
function foo(cb) {
  cb = function (a) {
    cb(1 + a);
  };
  bar(not_cb);
}
foo();
      