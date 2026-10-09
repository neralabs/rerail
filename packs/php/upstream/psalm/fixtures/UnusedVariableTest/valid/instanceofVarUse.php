<?php
                    interface Foo { }

                    function returnFoo(): Foo {
                        return new class implements Foo { };
                    }

                    $interface = Foo::class;

                    if (returnFoo() instanceof $interface) {
                        exit;
                    }