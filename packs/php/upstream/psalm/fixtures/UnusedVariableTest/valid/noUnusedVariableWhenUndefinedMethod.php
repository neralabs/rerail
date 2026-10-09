<?php
                    class A {}

                    function foo(A $a) : void {
                        $i = 0;

                        /** @psalm-suppress UndefinedMethod */
                        $a->bar($i);
                    }