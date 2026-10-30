
Object.defineProperty(obj, 'foo', {
  value: function () {
    console.log(this);
    z(x => console.log(x, this));
  },
});
      