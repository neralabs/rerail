
export interface Event<T> {
  (
    listener: (e: T) => any,
    thisArgs?: any,
    disposables?: Disposable[],
  ): Disposable;
}
      