<?php
                    /** @var non-empty-list<int> */
                    $arr = [1];
                    $arr[1] = &$arr[0];

                    takesArray($arr);

                    function takesArray(array $_arr): void {}
                