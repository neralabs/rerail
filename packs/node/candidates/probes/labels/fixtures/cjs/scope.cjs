const fs = require('fs');
module.exports = { dir: __dirname };
exports.file = __filename;
function load(require) {
  return require('x');
}
{
  const module = { id: 1 };
  module.id = 2;
}
const shorthand = { exports };
