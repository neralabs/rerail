<?php

                /** @param callable(string,int,bool,mixed,float): void $callable */
                function takesCallable(callable $callable): void
                {
                    /** @var mixed $mixed */
                    $mixed = null;
                    $callable("foo", 0, true, $mixed, 0.0);
                }

                takesCallable(
                    static function (string $foo, int $bar, $float) {
                        if ($bar === 0) {
                            throw new RuntimeException();
                        }
                    }
                );