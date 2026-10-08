<?php
                    /**
                     * @param list<int> $arr
                     */
                    function foo(array $arr) : void {
                        $a = false;

                        foreach ($arr as $b) {
                            $a = true;
                            echo $b;
                        }

                        echo $a;
                    }