
function test(file: Blob) {
  const slice: typeof file.slice =
    file.slice || (file as any).webkitSlice || (file as any).mozSlice;
  return slice;
}
    