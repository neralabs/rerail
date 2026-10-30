
nqz.efx = (function () {
  return function () {
    console.log(this);
    y(p => console.log(p, this));
  };
})();
      