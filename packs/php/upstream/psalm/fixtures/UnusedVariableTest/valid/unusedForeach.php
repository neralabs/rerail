<?php
                    /**
                     * @param array<int, string> $test
                     */
                    function foo(array $test) : void {
                        foreach($test as $key => $_testValue) {
                            echo $key;
                        }
                    }