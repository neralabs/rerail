class Members {
    constructor(
        readonly paramReadonly : number,
        public paramPublic : number,
        protected paramProtected : number,
        private paramPrivate : number,
        public readonly paramPublicReadonly : number,
        protected readonly paramProtectedReadonly : number,
        private readonly paramPrivateReadonly : number
    ){

    }

    constructor(readonly a : number, @decorated readonly b : number) {

    }
}
