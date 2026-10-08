<?php
                    /**
                     * @psalm-suppress MixedAssignment
                     * @psalm-suppress MixedArgument
                     * @param iterable<mixed, int> $keys
                     */
                    function foo(iterable $keys, int $colno) : void {
                        $i = 0;
                        $key = 0;
                        $index = 0;

                        foreach ($keys as $index => $key) {
                            if ($key === $colno) {
                                $i = $index;
                                break;
                            } elseif ($key > $colno) {
                                $i = $index;
                                break;
                            }
                        }

                        echo $i;
                        echo $index;
                        echo $key;
                    }