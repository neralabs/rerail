<?php
                    function f(callable $c): void {
                        $c(22);
                    }

                    f(
                        fn(int $p)
                            =>
                            0
                    );