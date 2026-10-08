// typescript-eslint's lib globals under `lib: ["dom"]`, in their namespaces.
type NodeListOf = 1;
var NodeList = 1;
interface HTMLElement { extra: 1 }
var local = 1;
let list: NodeList;
let ctor = Node;
let onlyType = NodeListOf;
let missing: NotALibType;
let promise: Promise<number> = Promise.resolve(1);
let unknownValue = notALibValue;
export {};
