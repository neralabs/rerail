if (false) {
    // @ts-ignore: Unreachable code error
    console.log("hello"); // Random comments.
}

var x = 0;

/// @ts-ignore Triple-slash
x();

/// @ts-ignore
x(
    2,
    3);

// come comment

// @ts-ignore Multiple comments

// some other comment

// @anohter

x();

x( // @ts-ignore inline
    2,
    3);
