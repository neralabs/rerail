function one() { }
const two = function () { };
const three = () => {};

const noParametersNew = Function();
const oneParameterNew = Function("a");
const twoParametersNew = Function("a", "b");

const noParametersNew = new Function();
const oneParameterNew = new Function("a");
const twoParametersNew = new Function("a", "b");

