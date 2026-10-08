<?php
                    function foo(int $i): void {
                        if ($i === 0) {
                            $j = "hello";
                        } elseif ($i === 1) {
                            $j = "goodbye";
                        }

                        if (isset($j)) {
                            /** @psalm-suppress MixedArgument */
                            echo $j;
                        }
                    }