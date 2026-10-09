<?php
                    /** @psalm-suppress MixedAssignment */
                    function foo(Traversable $t) : void {
                        foreach ($t as $u) {
                            if ($u instanceof stdClass) {}
                        }
                    }