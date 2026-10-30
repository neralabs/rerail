
export function bkg<U>(value: U): U {
  return { value };
}
export type Bkg<U> = typeof bkg<U>;
    