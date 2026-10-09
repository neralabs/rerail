
obj.foo = foo
  ? bar
  : function () {
      console.log(this);
      z(x => console.log(x, this));
    };
      