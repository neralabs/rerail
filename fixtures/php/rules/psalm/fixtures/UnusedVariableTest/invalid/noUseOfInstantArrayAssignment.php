<?php
                    function foo() : void {
                        /** @psalm-suppress PossiblyUndefinedVariable */
                        $arr["foo"] = 1;
                    }