
function foo<T extends (id: string, ...args: any[]) => any>(
  fn: T,
  ...args: any[]
) {}
    