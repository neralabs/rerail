<?php
                    function foo(string $a, object $b): void {
                        if ($a === "a") {
                            $hue = "hello";
                        } elseif ($a === "b") {
                            $hue = "goodbye";
                        }

                        /**
                         * @psalm-suppress PossiblyUndefinedVariable
                         * @psalm-suppress MixedMethodCall
                         */
                        $b->foo($hue);
                    }