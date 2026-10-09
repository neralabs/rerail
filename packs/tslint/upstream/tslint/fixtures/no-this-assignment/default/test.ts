var unscoped = this;

function testFunction() {
    let inFunction = this;
}

const testLambda = () => {
    const inLambda = this;
};

class TestClass {
    constructor() {
        const inConstructor = this;

        const asThis: this = this;

        const asString = "this";
        const asArray = [this];
        const asArrayString = ["this"];
    }

    public act(scope: this = this) {
        const inMemberFunction = this;

        const { act } = this;

        const { act, constructor } = this;

        const [foo] = this;

        const [foo, bar] = this;
    }
}

