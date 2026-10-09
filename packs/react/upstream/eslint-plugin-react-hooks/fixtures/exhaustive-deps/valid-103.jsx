
function useFoo(){
  let [foo] = [1];
  return useMemo(() => foo, [foo]);
}
