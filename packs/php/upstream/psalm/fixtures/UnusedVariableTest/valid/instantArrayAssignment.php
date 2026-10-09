<?php
                    function foo(string $b) : array {
                        /** @psalm-suppress PossiblyUndefinedVariable */
                        $arr["foo"] = $b;

                        return $arr;
                    }