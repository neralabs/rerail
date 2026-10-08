<?php
                    /** @psalm-suppress UnusedParam */
                    function foo(callable $c) : void {}
                    $listener = function () use (&$listener) : void {
                        foo($listener);
                    };
                    foo($listener);