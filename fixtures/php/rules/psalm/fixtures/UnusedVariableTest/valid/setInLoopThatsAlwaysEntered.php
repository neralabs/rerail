<?php
                    /**
                     * @param non-empty-array<int> $a
                     */
                    function getLastNum(array $a): int {
                        foreach ($a as $num) {
                            $last = $num;
                        }
                        return $last;
                    }