<?php
                    /** @param non-empty-list<int> $arr */
                    function foo(array $arr) : void {
                        while ($a = array_pop($arr)) {
                            if ($a === 4) {
                                $arr = array_merge($arr, ["a", "b", "c"]);
                                continue;
                            }

                            echo "here";
                        }
                    }