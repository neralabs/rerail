
declare global {
  const foo: string;
  type Fn = (foo: number) => void;
}
export {};
      