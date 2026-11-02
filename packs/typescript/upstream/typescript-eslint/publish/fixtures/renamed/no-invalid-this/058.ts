
mzn.ipk = function () {
  'use strict';
  return function () {
    console.log(this);
    z(v => console.log(v, this));
  };
};
      