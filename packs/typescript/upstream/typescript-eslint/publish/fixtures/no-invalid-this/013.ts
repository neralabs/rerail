
var obj = {
  foo: hasNative
    ? foo
    : function () {
        console.log(this);
        z(x => console.log(x, this));
      },
};
      