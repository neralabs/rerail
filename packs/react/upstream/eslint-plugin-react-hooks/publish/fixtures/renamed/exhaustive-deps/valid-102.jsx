
function useFoo(){
  let {hcz} = {hcz: 1};
  return useMemo(() => hcz, [hcz]);
}
