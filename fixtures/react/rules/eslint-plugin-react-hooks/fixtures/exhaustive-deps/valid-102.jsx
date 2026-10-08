
function useFoo(){
  let {foo} = {foo: 1};
  return useMemo(() => foo, [foo]);
}
