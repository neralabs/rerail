// Every function-like node is scored on its own; its decisions do not count toward the function around it.

export function outerWithComplexInner(values) {
  const label = (value) => {
    if (value === 1) {
      return "one";
    }
    if (value === 2) {
      return "two";
    }
    if (value === 3) {
      return "three";
    }
    if (value === 4) {
      return "four";
    }
    return value > 4 && value < 10 ? "few" : "many";
  };
  const pick = function (value) {
    return value > 0 && value < 100 && value !== 50 && value !== 60 && value !== 70 && value !== 80;
  };
  if (values.length === 0) {
    return [];
  }
  return values.filter(pick).map(label);
}

export function* generatorOverLimit(values) {
  for (const value of values) {
    if (value === 1) {
      yield 1;
    } else if (value === 2) {
      yield 2;
    } else if (value === 3) {
      yield 3;
    } else if (value === 4) {
      yield 4;
    } else if (value === 5) {
      yield 5;
    }
  }
}

export class Dispatcher {
  constructor(initial) {
    this.handlers = [];
    this.state = 0;
    if (initial > 0) {
      this.state = initial;
    } else if (initial < 0) {
      this.state = -initial;
    } else if (initial === 0) {
      this.state = 1;
    }
    while (this.state > 100) {
      this.state -= 100;
    }
    this.state = this.state > 50 && this.state < 60 ? 55 : this.state;
  }

  get level() {
    if (this.state === 1) {
      return "one";
    }
    if (this.state === 2) {
      return "two";
    }
    if (this.state === 3) {
      return "three";
    }
    return this.state > 3 || this.state < 0 ? "many" : "none";
  }

  set level(value) {
    switch (value) {
      case "one":
        this.state = 1;
        break;
      case "two":
        this.state = 2;
        break;
      case "three":
        this.state = 3;
        break;
      case "four":
        this.state = 4;
        break;
      case "five":
        this.state = 5;
        break;
      case "six":
        this.state = 6;
        break;
    }
  }

  *events() {
    if (this.state > 0) {
      yield "positive";
    }
  }

  onEvent = (event) => {
    if (event === "a") {
      return 1;
    }
    if (event === "b") {
      return 2;
    }
    if (event === "c") {
      return 3;
    }
    if (event === "d") {
      return 4;
    }
    return event === "e" || event === "f" || event === "g" ? 5 : 0;
  };
}
