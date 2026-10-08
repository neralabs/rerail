<?php
                    /** @param array<string> $key */
                    function foo(object $a, string $k) : string {
                        $sortA = "";

                        /** @psalm-suppress MixedOperand */
                        $sortA .= $a->$k;

                        return $sortA;
                    }