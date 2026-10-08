<?php
                    function foo(string $a): void {
                        if ($a === "a") {
                            $hue = "hello";
                        } elseif ($a === "b") {
                            $hue = "goodbye";
                        }

                        /**
                         * @psalm-suppress PossiblyUndefinedVariable
                         * @psalm-suppress MixedArgument
                         */
                        echo $hue;
                    }