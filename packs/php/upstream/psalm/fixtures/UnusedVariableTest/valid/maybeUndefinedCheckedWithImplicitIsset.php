<?php
                    function foo(array $arr) : void {
                        if (rand(0, 1)) {
                            $maybe_undefined = $arr;
                        }

                        /** @psalm-suppress MixedAssignment */
                        $maybe_undefined = $maybe_undefined ?? [0];

                        print_r($maybe_undefined);
                    }