<?php
                    /**
                     * @param non-empty-list<non-empty-list<int>> $arr
                     * @param-out non-empty-list<non-empty-list<int>> $arr
                     */
                    function foo(array &$arr): void {
                        $a = &$arr[0];
                        $b = &$a[0];
                        $b = 5;
                    }