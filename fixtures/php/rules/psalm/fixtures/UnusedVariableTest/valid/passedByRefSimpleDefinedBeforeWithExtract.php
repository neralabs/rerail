<?php
                    function foo(array $arr) : void {
                        while (rand(0, 1)) {
                            /** @psalm-suppress MixedArgument */
                            extract($arr);
                            $a = [];
                            takes_ref($a);
                        }
                    }

                    /**
                     * @param mixed $p
                     * @psalm-suppress UnusedParam
                     */
                    function takes_ref(&$p): void {}