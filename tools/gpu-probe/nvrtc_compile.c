// nvrtc_compile ARCH FILE: FILE (CUDA C++, already preprocessed) compiled by NVRTC for sm_ARCH, as the engine's
// CUDA backend will compile a UKL kernel (decision 0400 §6); prints the log, the entry points and the cubin's size.
// Build: cc -O2 -o nvrtc_compile nvrtc_compile.c -I/usr/local/cuda/include -L/usr/local/cuda/lib64 -lnvrtc -lcuda
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <nvrtc.h>
#include <cuda.h>

int main(int argc, char** argv) {
    if (argc < 3) { fprintf(stderr, "nvrtc_compile ARCH FILE\n"); return 2; }
    FILE* f = fopen(argv[2], "rb"); if (!f) { perror(argv[2]); return 2; }
    fseek(f, 0, SEEK_END); long n = ftell(f); fseek(f, 0, SEEK_SET);
    char* src = malloc(n + 1); fread(src, 1, n, f); src[n] = 0; fclose(f);
    nvrtcProgram prog;
    nvrtcCreateProgram(&prog, src, argv[2], 0, NULL, NULL);
    char arch[64]; snprintf(arch, sizeof arch, "--gpu-architecture=sm_%s", argv[1]);
    const char* opts[] = { arch, "--std=c++20", "-default-device" };
    nvrtcResult r = nvrtcCompileProgram(prog, 3, opts);
    size_t ls; nvrtcGetProgramLogSize(prog, &ls);
    char* log = malloc(ls + 1); nvrtcGetProgramLog(prog, log); log[ls] = 0;
    if (ls > 1) printf("%s\n", log);
    if (r != NVRTC_SUCCESS) { printf("NVRTC: %s\n", nvrtcGetErrorString(r)); return 1; }
    size_t cs; nvrtcGetCUBINSize(prog, &cs);
    char* cubin = malloc(cs); nvrtcGetCUBIN(prog, cubin);
    printf("cubin %zu bytes\n", cs);
    // Load it on the device: the entry points the module exports, by the names the shim gives them.
    cuInit(0); CUdevice d; cuDeviceGet(&d, 0); CUcontext c; cuCtxCreate(&c, NULL, 0, d);
    CUmodule m; CUresult lr = cuModuleLoadData(&m, cubin);
    if (lr != CUDA_SUCCESS) { const char* s; cuGetErrorString(lr, &s); printf("load: %s\n", s); return 1; }
    for (int k = 3; k < argc; ++k) {
        CUfunction fn; CUresult fr = cuModuleGetFunction(&fn, m, argv[k]);
        int regs = 0, smem = 0; if (fr == CUDA_SUCCESS) { cuFuncGetAttribute(&regs, CU_FUNC_ATTRIBUTE_NUM_REGS, fn); cuFuncGetAttribute(&smem, CU_FUNC_ATTRIBUTE_SHARED_SIZE_BYTES, fn); }
        printf("  %-36s %s regs %d static smem %d\n", argv[k], fr == CUDA_SUCCESS ? "found" : "MISSING", regs, smem);
    }
    return 0;
}
