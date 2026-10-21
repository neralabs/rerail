
type ExampleUnion = boolean | number;

type ExampleRoot = ExampleUnion | ExampleObject;

interface ExampleObject {
  [key: string]: ExampleRoot;
}
    