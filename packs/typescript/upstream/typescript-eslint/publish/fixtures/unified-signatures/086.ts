
namespace Namespace {
  export type Value = string;
}
function f(x: Namespace.Value): void;
function f(x: Namespace.Value | string): void;
      