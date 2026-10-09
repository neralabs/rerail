function doIt() {}
async function doAsync() {}
function print(strs) {}

[].forEach(x => console.log(x));

[].forEach(x => x && console.log(x));

async function f(): Promise<void> {
    const x = doIt();
    console.log(await doAsync());
    return print``;
}

true && console.log(0);
false || console.log(0);

declare function doIt(): void;
declare function doAsync(): Promise<void>;
declare function print(strs: TemplateStringsArray): void;

!!data ? console.info(message, data) : console.info(message);

