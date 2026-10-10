
type AST<T extends ParserOptions> = TSESTree.Program &
  (T['range'] extends true ? { range: [number, number] } : {}) &
  (T['tokens'] extends true ? { tokens: TSESTree.Token[] } : {}) &
  (T['comment'] extends true ? { comments: TSESTree.Comment[] } : {});
interface ParseAndGenerateServicesResult<T extends ParserOptions> {
  ast: AST<T>;
  services: ParserServices;
}
    