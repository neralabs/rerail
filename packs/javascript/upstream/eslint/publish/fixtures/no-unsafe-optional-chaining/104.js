async function foo() {
              (bar?.baz ?? await obj?.foo)();
            }