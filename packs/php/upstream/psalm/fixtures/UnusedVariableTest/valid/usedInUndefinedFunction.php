<?php
                    /**
                     * @psalm-suppress MixedReturnStatement
                     */
                    function test(): string {
                        $s = "a";
                        /** @psalm-suppress UndefinedFunction */
                        return undefined_function($s);
                    }