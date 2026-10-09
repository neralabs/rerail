<?php
                    /** @psalm-suppress MixedMethodCall */
                    function passesByRef(object $a): void {
                        /** @psalm-suppress PossiblyUndefinedVariable */
                        $a->passedByRef($b);
                    }