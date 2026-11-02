
rdi.ieq = (() =>
  function () {
    console.log(this);
    z(w => console.log(w, this));
  })();
      