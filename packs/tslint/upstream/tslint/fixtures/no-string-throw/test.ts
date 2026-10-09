let a: never = () => throw 'bla';

let b: never = () => throw new Error('bla');

let c: never = () => throw 'string1' + 'string2' + 'string3';

let d: never = () => throw 'string' + 1;

let e: never = () => throw 'string1' + 1 + {};

let f: never = () => throw ('string');

let g: never = () => throw 1 + 2 + ('string');

// no warning because rule does not check for toString()
const one = 1;
let h: never = () => throw one.toString();

let i: never = () => throw `some template string`;

const someVariable = 123;
let j: never = () => throw `template with ${someVariable} string`;

throw('component requires CSS height property');

throw'component...'

throw(('component...'))

throw/**/'component'
