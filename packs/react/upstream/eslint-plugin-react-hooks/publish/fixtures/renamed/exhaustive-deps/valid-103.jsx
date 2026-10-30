
function useFoo(){
  let [qme] = [1];
  return useMemo(() => qme, [qme]);
}
