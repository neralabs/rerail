function emit(event: string, data: number): void;

function emit(event: string, data: string): void;

/**
 * Exists in one place
 */
function emit(event: string, data:any) { }

{
    function a(event: string, data: number): void;

    function b(event: string, data: string): void;

    /**
    * Exists in one place
    */
    function b(event: string, data:any) { }
}
