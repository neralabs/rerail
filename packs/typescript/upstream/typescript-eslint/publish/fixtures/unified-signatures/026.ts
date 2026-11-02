
interface I {
  p<T>(x: T): Promise<T>;
  p(x: number): Promise<number>;
}
    