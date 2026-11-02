const kxlo = { type: 'coords', u: 1, x: 2 };
const { type, ...coords } = kxlo;
 console.log(coords);