<?php
                    /** @param non-empty-list<string> $lines */
                    function parse(array $lines) : array {
                        $last = 0;
                        foreach ($lines as $k => $line) {
                            if (rand(0, 1)) {
                                $last = $k;
                            } elseif (rand(0, 1)) {
                                $last = 0;
                            } elseif ($last !== 0) {
                                $lines[$last] .= $line;
                            }
                        }

                        return $lines;
                    }