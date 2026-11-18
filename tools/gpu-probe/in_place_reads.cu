// In-place reads on a unified-memory CUDA GPU (decision 0400, §2 and E1): what memory a kernel reads where it
// lives (cudaMalloc, malloc, a file mmap, an anonymous mmap, registered or not, huge pages), and how fast.
// Build and run on a CUDA host: nvcc -O2 -arch=sm_121 -w -o in_place_reads in_place_reads.cu && ./in_place_reads
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <sys/mman.h>
#include <fcntl.h>
#include <unistd.h>
#include <chrono>
#include <cuda_runtime.h>

#define CK(x) do { cudaError_t e = (x); if (e != cudaSuccess) { printf("%s: %s\n", #x, cudaGetErrorString(e)); } } while (0)

__global__ void sum(const unsigned long long* p, size_t n, unsigned long long* out) {
    unsigned long long s = 0;
    for (size_t i = blockIdx.x * (size_t)blockDim.x + threadIdx.x; i < n; i += (size_t)gridDim.x * blockDim.x) s += p[i];
    atomicAdd(out, s);
}

static double run(const char* what, const unsigned long long* p, size_t n, unsigned long long expect) {
    unsigned long long* out; CK(cudaMallocManaged(&out, 8));
    double best = 1e9;
    for (int r = 0; r < 4; ++r) {
        *out = 0;
        auto t0 = std::chrono::steady_clock::now();
        sum<<<48 * 8, 256>>>(p, n, out);
        cudaError_t e = cudaDeviceSynchronize();
        auto t1 = std::chrono::steady_clock::now();
        if (e != cudaSuccess) { printf("%-28s FAILED: %s\n", what, cudaGetErrorString(e)); cudaGetLastError(); return 0; }
        double s = std::chrono::duration<double>(t1 - t0).count();
        if (s < best) best = s;
    }
    printf("%-28s %s  best %.2f ms  %.1f GB/s\n", what, *out == expect ? "ok " : "BAD", best * 1e3, n * 8 / best / 1e9);
    cudaFree(out);
    return best;
}

int main() {
    cudaDeviceProp pr; CK(cudaGetDeviceProperties(&pr, 0));
    printf("%s sm_%d%d SMs %d integrated %d smem/block %zu smem/SM %zu regs/SM %d warp %d L2 %d MB\n", pr.name, pr.major, pr.minor,
           pr.multiProcessorCount, pr.integrated, pr.sharedMemPerBlock, pr.sharedMemPerMultiprocessor, pr.regsPerMultiprocessor,
           pr.warpSize, pr.l2CacheSize >> 20);
    int a;
    const struct { cudaDeviceAttr at; const char* n; } attrs[] = {
        {cudaDevAttrPageableMemoryAccess, "pageableMemoryAccess"},
        {cudaDevAttrPageableMemoryAccessUsesHostPageTables, "pageableUsesHostPageTables"},
        {cudaDevAttrConcurrentManagedAccess, "concurrentManagedAccess"},
        {cudaDevAttrCanUseHostPointerForRegisteredMem, "hostPtrForRegisteredMem"},
        {cudaDevAttrHostNativeAtomicSupported, "hostNativeAtomics"},
        {cudaDevAttrDirectManagedMemAccessFromHost, "directManagedFromHost"},
        {cudaDevAttrHostRegisterSupported, "hostRegisterSupported"},
        {cudaDevAttrCooperativeLaunch, "cooperativeLaunch"},
        {cudaDevAttrMemoryPoolsSupported, "memoryPools"},
    };
    for (auto& x : attrs) { CK(cudaDeviceGetAttribute(&a, x.at, 0)); printf("  %-28s %d\n", x.n, a); }
    size_t fr, tot; CK(cudaMemGetInfo(&fr, &tot)); printf("  memGetInfo free %zu MB total %zu MB\n", fr >> 20, tot >> 20);

    const size_t bytes = 256u << 20, n = bytes / 8;
    unsigned long long expect = 0;
    unsigned long long* h = (unsigned long long*)malloc(bytes);
    for (size_t i = 0; i < n; ++i) { h[i] = i; expect += i; }

    unsigned long long* d; CK(cudaMalloc(&d, bytes)); CK(cudaMemcpy(d, h, bytes, cudaMemcpyHostToDevice));
    run("cudaMalloc", d, n, expect);
    run("malloc (pageable, in place)", h, n, expect);

    const char* path = "probe.bin";
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0644); ftruncate(fd, bytes);
    void* m = mmap(nullptr, bytes, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    memcpy(m, h, bytes);
    run("mmap file (in place)", (const unsigned long long*)m, n, expect);
    CK(cudaHostRegister(m, bytes, cudaHostRegisterReadOnly));
    run("mmap file, HostRegister", (const unsigned long long*)m, n, expect);
    CK(cudaHostUnregister(m));
    munmap(m, bytes); close(fd); unlink(path);

    void* an = mmap(nullptr, bytes, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    memcpy(an, h, bytes);
    run("anon mmap (in place)", (const unsigned long long*)an, n, expect);
    CK(cudaHostRegister(an, bytes, 0));
    run("anon mmap, HostRegister", (const unsigned long long*)an, n, expect);

    void* hp = mmap(nullptr, bytes, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    printf("  madvise HUGEPAGE: %d\n", madvise(hp, bytes, MADV_HUGEPAGE));
    memcpy(hp, h, bytes);
    run("anon mmap THP (in place)", (const unsigned long long*)hp, n, expect);
    // CPU after GPU: does the CPU see a GPU write to pageable memory?
    unsigned long long* w = (unsigned long long*)malloc(64); w[0] = 0;
    sum<<<1, 1>>>(h, 4, w); CK(cudaDeviceSynchronize());
    printf("  GPU atomic into malloc: %llu (expect 6)\n", w[0]);
    return 0;
}
