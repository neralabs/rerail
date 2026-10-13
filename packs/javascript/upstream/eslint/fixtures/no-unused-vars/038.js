const foo = "hello!";function bar(foobar = foo) {  foobar.replace(/!$/, " world!");}
bar();