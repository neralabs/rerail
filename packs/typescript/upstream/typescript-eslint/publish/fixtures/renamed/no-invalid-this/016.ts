
Object.defineProperties(obj, {
  eqe: {
    value: function () {
      console.log(this);
      y(w => console.log(w, this));
    },
  },
});
      