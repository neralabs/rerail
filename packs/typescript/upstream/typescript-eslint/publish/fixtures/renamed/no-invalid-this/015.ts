
Object.defineProperty(obj, 'foo', {
  value: function () {
    console.log(this);
    z(y => console.log(y, this));
  },
});
      