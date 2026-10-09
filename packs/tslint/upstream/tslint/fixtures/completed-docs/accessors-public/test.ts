export class Foo {

    private myField: string;

    public get myAccessor(): string { return this.myField; }


    public set myAccessor(value: string) { this.myField = value; }

    private hiddenField: string;
    private get hiddenAccessor(): string { return this.hiddenField; }
    private set hiddenAccessor(value: string) { this.hiddenField = value; }

}
