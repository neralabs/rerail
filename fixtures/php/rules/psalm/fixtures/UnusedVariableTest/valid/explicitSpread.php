<?php
                    function f(): array {
                        $s = [1, 2, 3];
                        $b = ["a", "b", "c"];

                        $r = [...$s, ...$b];
                        return $r;
                    }