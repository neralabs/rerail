
        async function func() {
          await obj?.foo();
          await obj?.foo?.();
          (await obj?.foo)?.();
          (await obj?.foo)?.bar;
          await bar?.baz;
          await (foo ?? obj?.foo.baz);
          (await bar?.baz ?? bar).baz;
          (await bar?.baz ?? await bar).baz;
          await (foo?.bar ? baz : qux);
        }
        