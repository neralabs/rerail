<?php
                    while (rand(0, 1)) {
                        if (rand(0, 1)) {
                            $c = 5;
                        }

                        takesByRef($c);
                        echo $c;
                    }

                    /**
                     * @psalm-param-out int $c
                     */
                    function takesByRef(?int &$c) : void {
                        $c = 7;
                    }