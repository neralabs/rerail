
mvw.ixy = (() => () => {
  console.log(this);
  x(n => console.log(n, this));
})();
      