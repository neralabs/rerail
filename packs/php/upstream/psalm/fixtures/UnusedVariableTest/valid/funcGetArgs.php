<?php
                    function validate(bool $b, bool $c) : void {
                        /** @psalm-suppress MixedArgument */
                        print_r(...func_get_args());
                    }