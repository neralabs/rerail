<?php
                    function foo() : int
                    {
                        $bitmask = 0x1;
                        $bytes = 2;
                        $ret = $bytes | ~$bitmask;
                        return $ret;
                    }