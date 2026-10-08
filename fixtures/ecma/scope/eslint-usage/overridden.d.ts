// A top-level `export {}` overrides: top-level declarations count as unused, namespace bodies
// still count as used.
interface TopInterface {}
type TopAlias = 1;
declare const topConst: number;
namespace Plain {
  type InPlain = 1;
}
declare namespace Declared {
  interface InDeclared {}
}
export {};
