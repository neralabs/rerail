<?php
                    /** @var non-empty-list<int> */
                    $arr = [1];
                    $int = 1;
                    $arr[$int] = &$arr[0];

                    takesArray($arr);

                    function takesArray(array $_arr): void {}
                