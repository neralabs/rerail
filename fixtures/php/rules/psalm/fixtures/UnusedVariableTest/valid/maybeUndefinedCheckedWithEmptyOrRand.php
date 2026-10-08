<?php
                    function foo(array $arr) : void {
                        if (rand(0, 1)) {
                            $maybe_undefined = $arr;
                        }

                        if (empty($maybe_undefined) || rand(0, 1)) {
                            $maybe_undefined = [0];
                        }

                        print_r($maybe_undefined);
                    }