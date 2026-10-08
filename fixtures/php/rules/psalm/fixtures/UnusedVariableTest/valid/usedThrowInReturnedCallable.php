<?php
                    function createFailingFunction(RuntimeException $exception): Closure
                    {
                        return static function () use ($exception): void {
                            throw $exception;
                        };
                    }
                