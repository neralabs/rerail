<?php
                    /**
                     * @psalm-suppress MixedMethodCall
                     * @psalm-suppress MixedArgument
                     * @psalm-suppress PossiblyNullArgument
                     * @param mixed $mixed
                     * @param mixed|null $mixed_or_null
                     */
                    function foo($mixed, $mixed_or_null): void {
                        $mixed->foo(new Exception($mixed_or_null));
                    }