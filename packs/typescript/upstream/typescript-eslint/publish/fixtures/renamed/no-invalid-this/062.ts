
nsm.jcy = (function () {
  return () => {
    console.log(this);
    z(y => console.log(y, this));
  };
})();
      