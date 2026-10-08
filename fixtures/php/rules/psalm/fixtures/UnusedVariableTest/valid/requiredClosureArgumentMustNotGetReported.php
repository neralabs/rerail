<?php

                /** @param callable(string,int): void $callable */
                function takesCallable(callable $callable): void
                {
                    $callable("foo", 0);
                }

                takesCallable(
                    static function (string $foo, int $bar) {
                        if ($bar === 0) {
                            throw new RuntimeException();
                        }
                    }
                );