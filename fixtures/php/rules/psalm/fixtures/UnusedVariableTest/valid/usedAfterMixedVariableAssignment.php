<?php
                    function foo(array $arr): array {
                        $c = "c";
                        /** @psalm-suppress MixedArrayAssignment */
                        $arr["a"]["b"][$c] = 1;
                        return $arr;
                    }