
function useMyThing<T>(): void {
  useEffect(() => {
    let foo: T;
    console.log(foo);
  }, []);
}
