var x = parseInt(3, 10) +
        parseInt(4);
var y = parseFloat("2.5");

parser.parseInt("123");
obj.parser.parseInt("123");

global.parseInt("123");
global.global.parseInt("123");
window.parseInt("123");

Number.parseInt("123", 10);
Number.parseInt("123");
other.Number.parseInt("123");

global.Number.parseInt("123", 10);
global.Number.parseInt("123");
other.global.Number.parseInt("123");

window.Number.parseInt("123", 10);
window.Number.parseInt("123");
other.window.Number.parseInt("123");
