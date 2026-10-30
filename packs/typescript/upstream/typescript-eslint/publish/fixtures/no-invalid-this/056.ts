
var obj = {
  foo: function () {
    'use strict';
    return function () {
      console.log(this);
      z(x => console.log(x, this));
    };
  },
};
      