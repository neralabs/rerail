async function func() {
            await obj?.foo + await obj?.bar;
            await obj?.foo - await obj?.bar;
            await obj?.foo * await obj?.bar;
            +await obj?.foo;
            -await obj?.foo;
            bar += await obj?.foo;
            bar -= await obj?.foo;
            bar %= await obj?.foo;
            bar **= await obj?.foo;
            bar *= await obj?.boo;
            bar /= await obj?.boo;
        }
        