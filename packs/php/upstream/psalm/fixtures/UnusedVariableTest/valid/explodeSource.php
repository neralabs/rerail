<?php
                    $start = microtime();
                    $start = explode(" ", $start);
                    /**
                     * @psalm-suppress InvalidOperand
                     */
                    $start = $start[1] + $start[0];
                    echo $start;