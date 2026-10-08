<?php
                    function foo(string $c): void {
                        $arr = [$c];
                        $arr[] = 1;

                        foreach ($arr as $e) {
                            echo $e;
                        }
                    }