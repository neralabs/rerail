<?php
                    function bar(): int {
                        if (rand(0, 1) === 0) {
                            $foo = 0;
                        } elseif ($foo = rand(0, 10)) {
                            return 5;
                        }

                        return $foo;
                    }