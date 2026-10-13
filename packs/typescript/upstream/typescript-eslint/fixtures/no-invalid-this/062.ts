
obj.foo = (function () {
  return () => {
    console.log(this);
    z(x => console.log(x, this));
  };
})();
      