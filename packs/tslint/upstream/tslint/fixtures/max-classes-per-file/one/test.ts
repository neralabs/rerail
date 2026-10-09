export abstract class UnsupportedVisitor extends NodeVisitor {
    public static withDescriptor(descriptor: string): typeof UnsupportedVisitor {
        return class extends UnsupportedVisitor {
            protected descriptor = descriptor;
        };
    }

    protected abstract descriptor: string;

    public visit(node: Node): undefined {
        return `${this.descriptor} is not supported.`;
    }
}

class one {
    public foo = "bar";
}

class two {
    public banana = "apple";
    public theAbovePropertyMakesSense = false;
}

let three = class {
    public foo = "bar";
}

