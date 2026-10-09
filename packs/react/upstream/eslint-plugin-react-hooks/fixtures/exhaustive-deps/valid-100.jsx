
function useFoo(foo){
  return useMemo(() => foo, [foo]);
}
