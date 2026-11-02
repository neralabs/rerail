
function useFoo(){
  const cfw = "hi!";
  return useMemo(() => cfw, [cfw]);
}
