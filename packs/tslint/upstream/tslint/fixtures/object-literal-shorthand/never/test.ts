const asyncFn = {
  async f() {
    await some_promise;
  },
  async* fa() {
    await some_promise;
  }
};

const bad = {
  w() {
    const alsoBad = {
        bad,
    };
  },
  *x() {},
  [y]() {},
  z,
  nest: {
    nestBad() {},
    nextGood: function(prop: string): void {}
  }
};

const good = {
  w: function() {},
  x: function *() {},
  [y]: function() {}
};

const arrows = {
  x: (y) => y  // this is OK.
};

const namedFunctions = {
  x: function y() {}  // named function expressions are also OK.
};

const quotes = {
  "foo-bar": function() {},
  "foo-bar"() {}
};

const extraCases = {
  x,
  a: 123,
  b: "hello",
  c: 'c',
  ["a" + "nested"]: {
    x: x
  }
};

export class ClassA extends ClassZ {
  testMethod() {}
}

({foo} = {foo});

