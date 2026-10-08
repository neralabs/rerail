// typescript-eslint's declaration files: every top-level declaration, and every declaration in a
// namespace body, counts as used unless its container re-exports (`export =`, `export {}`, ...).
interface TopInterface {}
type TopAlias = 1;
declare class TopClass {}
declare function topFunction(a: number): void;
declare enum TopEnum { A }
declare const topConst: number, topOther: string;
declare let { destructured }: { destructured: number };
declare namespace TopSpace {
  interface InSpace {}
  const inSpace: number;
  namespace Nested {
    type Deep = 1;
  }
}
namespace Plain {
  type InPlain = 1;
}
namespace Overridden {
  type Shadow = 1;
  type Kept = 1;
  export = Kept;
}
namespace AllStar {
  type Starred = 1;
  export * from "m";
}
namespace ByDefault {
  type Defaulted = 1;
  const other: number;
  export default other;
}
declare module "m" {
  type InModule = 1;
}
export interface Exported {}
