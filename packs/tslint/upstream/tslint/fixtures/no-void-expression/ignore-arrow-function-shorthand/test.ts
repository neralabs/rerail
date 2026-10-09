[].forEach(x => console.log(x));
[].forEach(x => x && console.log(x));
[].forEach(x => !x || console.log(x));

[].forEach(x => {
    return console.log(x);
});

