<?php
                    /**
                     * @param array<string, int> $row
                     */
                    function foo(array $row, string $s) : array {
                        $row["a" . strtolower($s)] += 1;
                        return $row;
                    }