const array = [1, 2, 3, 4];
const strArray = ['a', 'b', 'c'];
const objArray = [{a: 1}, {a: 2, b: 10}, {a: 3}];
const o = {};
const numArray = {
    1: "a",
    2: "b"
};
const d = new Date();
const str = "abc";
const objectMap = {} as {[key: string]: number};
const map = new Map([['a', 1], ['b', 2]]);
const set = new Set([1, 2, 3]);

class C {
  a: number;
  f() {}
  g() {}
}
c = new C();

class A<T> { }
let refType: A<number>;

// valid cases
for (var k in o);
for (var k in d);
for (var k in map);
for (var k in numArray);
for (var k in c);
for (var k in refType);

// invalid cases
for (var k in array);
for (var k in str);
for (var k in objArray);

// Ideally these would also be forbidden:
for (var k in map);
for (var k in set);

