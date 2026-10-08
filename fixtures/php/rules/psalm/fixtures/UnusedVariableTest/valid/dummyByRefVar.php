<?php
                    function foo(string &$a = null, string $b = null): void {
                        if ($a !== null) {
                            echo $a;
                        }
                        if ($b !== null) {
                            echo $b;
                        }
                    }

                    function bar(): void {
                        foo($dummy_byref_var, "hello");
                    }

                    bar();