<?php
                    function foo(array &$arr): void {
                        $b = 5;
                        $arr[0] = $b;
                    }