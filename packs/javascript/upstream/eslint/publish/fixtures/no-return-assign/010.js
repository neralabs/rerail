function bar(){
            return function foo(){
                return (a = b) && c
            }
        }