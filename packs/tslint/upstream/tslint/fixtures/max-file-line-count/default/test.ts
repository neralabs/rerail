namespace App {
    class GiveMeANumber {

        /* tslint:disable:no-any */
        private num:any = 10;
        /* tslint:enable:no-any */

        //This is some kind of comment
        constructor(){
            this.giveNumber();
        }

        /*
        Here is another comment
        It's very helpful.
        */

        public giveNumber = () =>{  
            alert(this.num);
        };
    }
}