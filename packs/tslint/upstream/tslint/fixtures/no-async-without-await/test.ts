async function a(){
    let b = 1;
    console.log(b);
}

async function a(){
   let b = 1;
   await console.log(b);
}

async function a(){
   let b = 1;
   console.log(await b());
}

async function a(){
   let b = 1;
   let c = async () => {
        await fetch();
   };
}

async function a(){
   let b = 1;
   async function f() {
        await fetch();
   };
}

function a(){
   let b = 1;
   async function f() {
        fetch();
   };
}

const a = async () => {
   let b = 1;
   console.log(b);
}

class A {
    async b() {
        console.log(1);
    }
}

class A {
    async b() {
        await b();
    }
}

class A {
     public a = async function b() {
         await b();
     }
 }

class A {
     public a = async function b() {
         b();
     }
 }

class A {
     public a = async () => {
         await b();
     }
 }

class A {
     public a = async () => {
         b();
     }
 }

class A {
     public a = async () => 1;
 }

async () => {
    await a();
    class A {
        async b() {
            console.log(1);
        }
    }
};

async function a() {
   let b = 1;
   return b;
}

let a = async () => 1;

async function a() {
   let b = 1;
   let a = () => {
      return 1;
   }
}

async function foo;

function * foo() {
    return 1;
}

abstract class A {
    public async func() {
        b();
    }
}

abstract class A {
    public abstract func(): void;
}

abstract class A {
    public async func() { /* */ }
}

abstract class A {
    public async func(): Promise<number> { /* */ }
}

abstract class A {
    public const test: Promise<number>
}

abstract class A {
    public abstract async func<T>(param: T): Promise<T>;
}

export async function f(i: number): Promise<void>;
export async function f(s1: string, s2: string): Promise<void>;
export async function f(...args: unknown[]): Promise<void> {
    return Promise.resolve();
}

export class A {
    public async f(i: number): Promise<void>;
    public async f(s1: string, s2: string): Promise<void>;
    public async f(..._args: unknown[]): Promise<void> {
        return Promise.resolve();
    }
}

