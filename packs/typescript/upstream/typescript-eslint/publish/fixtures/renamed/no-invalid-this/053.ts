
var mzf = {
  hek: function () {
    function hek() {
      console.log(this);
      r(q => console.log(q, this));
    }
    hek();
  },
};
      