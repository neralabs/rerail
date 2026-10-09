class BadClass {
    badDefaultProperty;

    public badPublicProperty;

    protected badProtectedProperty;

    private badPrivateProperty;

    badDefaultMethod() { }

    public badPublicMethod() { }

    protected badProtectedMethod() { }

    private badPrivateMethod() { }

    get badDefaultAccessor() { return this.badDefaultProperty; }

    set badDefaultAccessor(value) { this.badDefaultProperty = value; }

    public get badPublicAccessor() { return this.badPublicProperty; }

    public set badPublicAccessor(value) { this.badPublicProperty = value; }

    protected get badProtectedAccessor() { return this.badProtectedProperty; }

    protected set badProtectedAccessor(value) { this.badProtectedProperty = value; }

    private get badPrivateAccessor() { return this.badPrivateProperty; }

    private set badPrivateAccessor(value) { this.badPrivateProperty = value; }
}

/**
 *
 */
class EmptyClass {
    /**
     *
     */
    emptyDefaultProperty;

    /**
     *
     */
    public emptyPublicProperty;

    /**
     *
     */
    protected emptyProtectedProperty;

    /**
     *
     */
    private emptyPrivateProperty;

    /**
     *
     */
    emptyDefaultMethod() { }

    /**
     *
     */
    public emptyPublicMethod() { }

    /**
     *
     */
    protected emptyProtectedMethod() { }

    /**
     *
     */
    private emptyPrivateMethod() { }

    /**
     *
     */
    get emptyDefaultAccessor() { return this.emptyDefaultProperty; }

    /**
     *
     */
    set emptyDefaultAccessor(value) { this.emptyDefaultProperty = value; }

    /**
     *
     */
    public get emptyPublicAccessor() { return this.emptyPublicProperty; }

    /**
     *
     */
    public set emptyPublicAccessor(value) { this.emptyPublicProperty = value; }

    /**
     *
     */
    protected get emptyProtectedAccessor() { return this.emptyProtectedProperty; }

    /**
     *
     */
    protected set emptyProtectedAccessor(value) { this.emptyProtectedProperty = value; }

    /**
     *
     */
    private get emptyPrivateAccessor() { return this.emptyPrivateProperty; }

    /**
     *
     */
    private set emptyPrivateAccessor(value) { this.emptyPrivateProperty = value; }
}

/**
 * ...
 */
class GoodClass {
    /**
     * ...
     */
    goodDefaultProperty;

    /**
     * ...
     */
    public goodPublicProperty;

    /**
     * ...
     */
    protected goodProtectedProperty;

    /**
     * ...
     */
    private goodPrivateProperty;

    /**
     * ...
     */
    goodDefaultMethod() { }

    /**
     * ...
     */
    public goodPublicMethod() { }

    /**
     * ...
     */
    protected goodProtectedMethod() { }

    /**
     * ...
     */
    private goodPrivateMethod() { }

    /**
     * ...
     */
    get goodDefaultAccessor() { return this.goodDefaultProperty; }

    /**
     * ...
     */
    set goodDefaultAccessor(value) { this.goodDefaultProperty = value; }

    /**
     * ...
     */
    public get goodPublicAccessor() { return this.goodPublicProperty; }

    /**
     * ...
     */
    public set goodPublicAccessor(value) { this.goodPublicProperty = value; }

    /**
     * ...
     */
    protected get goodProtectedAccessor() { return this.goodProtectedProperty; }

    /**
     * ...
     */
    protected set goodProtectedAccessor(value) { this.goodProtectedProperty = value; }

    /**
     * ...
     */
    private get goodPrivateAccessor() { return this.goodPrivateProperty; }

    /**
     * ...
     */
    private set goodPrivateAccessor(value) { this.goodPrivateProperty = value; }
}

enum BadEnum { }

/**
 *
 */
enum EmptyEnum { }

/**
 * ...
 */
enum GoodEnum { }

/**
 * ...
 */
enum EnumWithMembers {
    BadEnumMember

    /**
     * ...
     */
    GoodEnumMember
}

function BadFunction() { }

/**
 *
 */
function EmptyFunction() { }

/**
 * ...
 */
function GoodFunction() { }

interface BadInterface { }

/**
 *
 */
interface EmptyInterface { }

/**
 * ...
 */
interface GoodInterface { }

/**
 * ...
 */
interface BadInterfaceMembers {
    prop: number;
    method(): number;
}

/**
 * ...
 */
interface EmptyInterfaceMembers {
    /**
     *
     */
    prop: number;
    /**
     *
     */
    method(): number;
}

/**
 * ...
 */
interface GoodInterfaceMembers {
    /**
     * ...
     */
    prop: number;
    /**
     * ...
     */
    method(): number;
}

namespace BadNamespace { }

/**
 *
 */
namespace EmptyNamespace { }

/**
 * ...
 */
namespace GoodNamespace { }

type BadType = 1;

/**
 *
 */
type EmptyType = 1;

/**
 * ...
 */
type GoodType = 1;

type BadType = 1;

/**
 *
 */
type EmptyType = 1;

/**
 * ...
 */
type GoodType = 1;

const BadVariable = 1;

/**
 *
 */
const EmptyVariable = 1;

/**
 * ...
 */
const GoodVariable = 1;

/**
 * Properties, methods and accessors in object literals should not require documentation.
 */
let literal = {
    prop: 1,
    get accessor() { return literal.prop; },
    set accessor(value) { literal.prop = value; }
    someMethod() {}
}
