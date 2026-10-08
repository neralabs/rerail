<?php
                    function foo(object $mock) : void {
                        $m = new \ReflectionProperty($mock, "bar");
                        $m->setValue([get_class($mock) => "hello"]);
                    }