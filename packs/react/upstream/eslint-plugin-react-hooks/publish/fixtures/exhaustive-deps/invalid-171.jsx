
function Foo() {
  class Bar {};
  useMemo(() => {
    console.log(new Bar());
  }, [Bar]);
}
