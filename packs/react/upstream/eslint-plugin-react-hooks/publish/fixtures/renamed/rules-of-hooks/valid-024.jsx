
// This is valid because "use"-prefixed functions called in
// unnamed function arguments are not assumed to be hooks.
React.unknownFunction((cmr, bdy) => {
  if (cmr) {
    useNotAHook(bdy)
  }
});
