var x = null; // error
console.log(null, x); // error

let match: string | null;
interface foo {
    bar: [number, null, string];
}

if (document.querySelector('.foo') === null) {}
if (document.querySelector('.foo') == null) {}
if (null !== document.querySelector('.foo')) {}
if (null != document.querySelector('.foo')) {}
