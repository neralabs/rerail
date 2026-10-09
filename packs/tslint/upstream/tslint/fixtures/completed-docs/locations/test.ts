class Class {
    badInstanceProperty = 1;

    /**
     * ...
     */
    goodInstanceProperty = 1;

    /**
     * ...
     */
    static badStaticProperty = 1;

    static goodStaticProperty = 1;

    badInstanceMethod() { }

    /**
     * ...
     */
    goodInstanceMethod() { }

    static badStaticMethod() { }

    /**
     * ...
     */
    static goodStaticMethod() { }

    /**
     * ...
     */
    async goodAsyncMethod() { }

    async badAsyncMethod() { }

    /**
     * ...
     */
    private async goodPrivateAsyncMethod() { }

    private async badPrivateAsyncMethod() { }
}