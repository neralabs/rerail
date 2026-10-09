declare class AmbientNoAccess {
    a(): number;

    static b(): number;
}

declare class AmbientAccess {
    public a(): number;
    public static b(): number;
}

class Members {
    [x: string]: number;
    i: number;
    static j: number;

    public nPublic: number;
    protected nProtected: number;
    private nPrivate: number;

    public static nsPublic: number;
    protected static nsProtected: number;
    private static nsPrivate: number;

    noAccess(x: number): number;
    noAccess(o: any): any {}

    static noAccess(x: number): number;
    static noAccess(o: any): any {}

    public access(x: number): number;
    public access(o: any): any {}

    public static access(x: number): number;
    public static access(o: any): any {}
}

const obj = {
    func() {}
};

function main() {
    class A {
        i: number;
        static j: number;
        public n: number;
    }
}

abstract class Decorated {
    @decorator prop;
    @decorator abstract method() {}
    @decorator @moreDecorator({}) readonly static PROP = 'FOO';
}
