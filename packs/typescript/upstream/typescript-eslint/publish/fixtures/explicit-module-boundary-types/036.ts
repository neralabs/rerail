
export function foo(): unknown {
  return new Proxy(apiInstance, {
    get: (target, property) => {
      // implementation
    },
  });
}
    