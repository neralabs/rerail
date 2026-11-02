
nsh.dqt = function () {
  return function () {
    console.log(this);
    z(p => console.log(p, this));
  };
};
      