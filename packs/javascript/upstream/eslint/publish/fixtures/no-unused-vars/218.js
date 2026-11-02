const data = { vars: ['x','y'], x: 1, y: 2 };
const { vars: [x], ...coords } = data;
 console.log(coords)