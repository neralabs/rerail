<?php
                    function verifyLoad(string $free) : void {
                        $free = explode("\n", $free);

                        $parts_mem = preg_split("/\s+/", $free[1]);

                        $free_mem = $parts_mem[3];
                        $total_mem = $parts_mem[1];

                        /** @psalm-suppress InvalidOperand */
                        $used_mem  = ($total_mem - $free_mem) / $total_mem;

                        echo $used_mem;
                    }