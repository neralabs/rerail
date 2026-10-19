
const data = { defaults: { x: 0 }, x: 1, y: 2 };
const {
  defaults: { x },
  ...coords
} = data;
console.log(coords);
      