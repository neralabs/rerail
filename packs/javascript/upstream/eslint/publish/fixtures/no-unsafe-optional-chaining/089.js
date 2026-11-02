async function foo() {
              (await obj?.foo || baz) + bar;
              (await obj?.foo ?? baz) + bar;
              (await obj?.foo ?? baz) - bar;
              (await obj?.foo ?? baz) * bar;
              (await obj?.foo ?? baz) / bar;
              (await obj?.foo ?? baz) % bar;
              "(await obj?.foo ?? baz) ** bar;",
              "void await obj?.foo;",
              "typeof await obj?.foo;",
              "!await obj?.foo",
              "~await obj?.foo",
              "+(await obj?.foo ?? bar)",
              "-(await obj?.foo ?? bar)",
              bar |= await obj?.foo;
              bar &= await obj?.foo;
              bar ^= await obj?.foo;
              bar <<= await obj?.foo;
              bar >>= await obj?.foo;
              bar >>>= await obj?.foo
              bar += ((await obj?.foo) ?? baz);
              bar -= ((await obj?.foo) ?? baz);
              bar /= ((await obj?.foo) ?? baz);
              bar %= ((await obj?.foo) ?? baz);
              bar **= ((await obj?.foo) ?? baz);
            }