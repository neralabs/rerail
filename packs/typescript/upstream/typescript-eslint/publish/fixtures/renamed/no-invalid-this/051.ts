
return function () {
  console.log(this);
  s(m => console.log(m, this));
};
      