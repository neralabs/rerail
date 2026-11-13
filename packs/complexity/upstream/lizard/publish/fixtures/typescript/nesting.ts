// Every function-like node is scored on its own; its decisions do not count toward the function around it.

export function outerWithComplexInner(values: number[]): string[] {
  const label = (value: number): string => {
    if (value === 1) {
      return "one";
    }
    if (value === 2) {
      return "two";
    }
    if (value === 3) {
      return "three";
    }
    return value > 3 && value < 10 ? "few" : "many";
  };
  const pick = function (value: number): boolean {
    return value > 0 && value < 100 && value !== 50 && value !== 60 && value !== 70 && value !== 80;
  };
  if (values.length === 0) {
    return [];
  }
  return values.filter(pick).map(label);
}

export function* generatorOverLimit(values: number[]): Generator<number> {
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
  private readonly handlers: Array<(event: string) => number> = [];
  private state = 0;

  constructor(initial: number) {
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

  get level(): string {
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

  set level(value: string) {
    if (value === "one") {
      this.state = 1;
    } else if (value === "two") {
      this.state = 2;
    } else if (value === "three") {
      this.state = 3;
    } else if (value === "four") {
      this.state = 4;
    } else if (value === "five") {
      this.state = 5;
    } else if (value === "six") {
      this.state = 6;
    }
  }

  dispatch(event: string): number {
    for (const handler of this.handlers) {
      const result = handler(event);
      if (result > 0) {
        return result;
      }
    }
    return event.length > 0 ? 1 : 0;
  }

  *events(): Generator<string> {
    if (this.state > 0) {
      yield "positive";
    }
  }

  onEvent = (event: string): number => {
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

export const handlers = {
  parse(text: string): number {
    if (text === "") {
      return 0;
    }
    return text.length;
  },
};
