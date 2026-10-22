
type ObjectWithCallback = { callback: () => void };

const f = (gotcha: ObjectWithCallback = { callback: () => {} }): void => {};
      