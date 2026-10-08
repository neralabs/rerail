<?php
                    function test(Exception $e): callable {
                        return fn() => $e->getMessage();
                    }