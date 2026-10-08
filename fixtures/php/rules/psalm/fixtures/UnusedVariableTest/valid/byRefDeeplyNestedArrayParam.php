<?php
                    /**
                     * @param non-empty-list<non-empty-list<int>> $arr
                     * @param-out non-empty-list<non-empty-list<int>> $arr
                     */
                    function foo(array &$arr): void {
                        $b = 5;
                        $arr[0][0] = $b;
                    }