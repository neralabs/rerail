/** compute_kernel_grid.cc                                                -*- C++ -*-
    Jeremy Barnes, 27 March 2016
    This file is part of MLDB. Copyright 2016 mldb.ai inc. All rights reserved.

    Compute kernel runtime for CPU devices.
*/

#include "compute_kernel_grid.h"
#include "mldb/types/basic_value_descriptions.h"
#include "mldb/types/meta_value_description.h"
#include "mldb/types/structure_description.h"
#include "mldb/types/set_description.h"
#include "mldb/types/generic_array_description.h"
#include "mldb/types/generic_atom_description.h"
#include "mldb/vfs/filter_streams.h"
#include "mldb/utils/environment.h"
#include "mldb/arch/ansi.h"
#include "mldb/block/zip_serializer.h"
#include "mldb/utils/command_expression_impl.h"
#include "mldb/arch/spinlock.h"
#include "mldb/arch/timers.h"
#include <compare>

using namespace std;

namespace MLDB {

DEFINE_ENUM_DESCRIPTION_INLINE(GridComputeFunctionArgumentDisposition)
{
    addValue("BUFFER", GridComputeFunctionArgumentDisposition::BUFFER, "");
    addValue("LITERAL", GridComputeFunctionArgumentDisposition::LITERAL, "");
    addValue("THREADGROUP", GridComputeFunctionArgumentDisposition::THREADGROUP, "");
}

DEFINE_STRUCTURE_DESCRIPTION_INLINE(GridComputeFunctionArgument)
{
    addField("name", &GridComputeFunctionArgument::name, "");
    addField("computeFunctionArgIndex", &GridComputeFunctionArgument::computeFunctionArgIndex, "");
    addField("type", &GridComputeFunctionArgument::type, "");
    addField("disposition", &GridComputeFunctionArgument::disposition, "");
    addField("implInfo", &GridComputeFunctionArgument::implInfo, "");
}

EnvOption<int> GRID_TRACE_API_CALLS("GRID_COMPUTE_TRACE_API_CALLS", 0);

bool gridTraceApiCalls()
{
    return GRID_TRACE_API_CALLS.get();
}

namespace {

std::mutex kernelRegistryMutex;
struct KernelRegistryEntry {
    std::function<std::shared_ptr<GridComputeKernelTemplate>(GridComputeContext & context)> generate;
};

std::map<std::string, KernelRegistryEntry> kernelRegistry;

EnvOption<std::string, true> GRID_KERNEL_TRACE_FILE("GRID_KERNEL_TRACE_FILE", "");

std::shared_ptr<ZipStructuredSerializer> traceSerializer;
std::shared_ptr<StructuredSerializer> regionsSerializer;
std::shared_ptr<StructuredSerializer> kernelsSerializer;
std::shared_ptr<StructuredSerializer> programsSerializer;

using TracedRegionKey = std::tuple<std::string, int>;

struct TracedRegionEntry {
    size_t length = 0;
};

std::mutex tracedRegionMutex;
std::map<std::string, std::shared_ptr<StructuredSerializer>> regionSerializers;
std::map<TracedRegionKey, TracedRegionEntry> tracedRegions;

std::mutex tracedProgramMutex;
std::set<std::string> tracedPrograms;

bool versionIsTraced(const std::string & name, int version)
{
    std::unique_lock guard(tracedRegionMutex);
    return tracedRegions.count({name, version});
}

void traceVersion(std::string name, int version, const FrozenMemoryRegion & region)
{
    if (name == "") {
        name = format("%016llx", (unsigned long long)region.data());
    }
    ExcAssert(name != "");
    ExcAssertGreaterEqual(version, 0);

    if (!traceSerializer)
        return;

    std::unique_lock guard(tracedRegionMutex);
    auto it = tracedRegions.find({name, version});
    if (it != tracedRegions.end()) {
        ExcAssertEqual(it->second.length, region.length());
        return;
    }

    if (!regionSerializers.count(name)) {
        regionSerializers[name] = regionsSerializer->newStructure(name);
    }
    auto thisRegionSerializer = regionSerializers[name];
    ExcAssert(thisRegionSerializer);
    thisRegionSerializer->addRegion(region, version);

    tracedRegions[{name, version}] = {region.length()};
}

struct InitTrace {
    InitTrace()
    {
        if (GRID_KERNEL_TRACE_FILE.specified()) {
            traceSerializer = std::make_shared<ZipStructuredSerializer>(GRID_KERNEL_TRACE_FILE.get());
            regionsSerializer = traceSerializer->newStructure("regions");
            kernelsSerializer = traceSerializer->newStructure("kernels");
            programsSerializer = traceSerializer->newStructure("programs");
        }
    }

    ~InitTrace()
    {
        if (traceSerializer) {
            regionsSerializer->commit();
            kernelsSerializer->commit();
            traceSerializer->commit();
            programsSerializer->commit();
        }
    }
} initTrace;

__thread int opCount = 0;
Timer startTimer;
double lastTime = 0;

static std::string printTime(double elapsed)
{
    std::string timerStr;
    if (elapsed < 0.000001) {
        timerStr = format("%.1fns", elapsed * 1000000000.0);
    }
    else if (elapsed < 0.001) {
        timerStr = format("%.1fus", elapsed * 1000000.0);
    }
    else if (elapsed < 1) {
        timerStr = format("%.1fms", elapsed * 1000.0);
    }
    else {
        timerStr = format("%.1fs", elapsed);
    }
    return timerStr;
}

} // file scope


void traceOperationImpl(OperationScope opScope, OperationType opType, const std::string & opName,
                        const std::string & renderedArgs)
{
    if (GRID_TRACE_API_CALLS.get()) {
        using namespace MLDB::ansi;
        int tid = std::hash<std::thread::id>()(std::this_thread::get_id());
        double elapsed = startTimer.elapsed_wall();

        std::string opTypeName;
        std::string opColor;

        switch (opType) {
        case OperationType::GRID_COMPUTE:
            opTypeName = "GRID:";
            opColor = ansi::ansi_str_cyan();
            break;
        case OperationType::METAL_COMPUTE:
            opTypeName = "METAL:";
            opColor = ansi::ansi_str_blue();
            break;
        case OperationType::OPENCL_COMPUTE:
            opTypeName = "OPENCL:";
            opColor = ansi::ansi_str_green();
            break;
        case OperationType::CPU_COMPUTE:
            opTypeName = "CPU:";
            opColor = ansi::ansi_str_yellow();
            break;
        case OperationType::CUDA_COMPUTE:
            opTypeName = "CUDA:";
            opColor = ansi::ansi_str_blue();
            break;
        case OperationType::USER:
            opTypeName = "USER:";
            opColor = ansi::ansi_str_bright_blue();
            break;
        }

        std::string opPrefix;

        switch (opScope) {
        case OperationScope::ENTER:
            opPrefix = "BEGIN ";
            break;

        case OperationScope::EVENT:
            opPrefix = "";
            break;

        case OperationScope::EXIT:
            opPrefix = "END ";
            break;

        case OperationScope::EXIT_EXC:
            opPrefix = "EXCEPTION ";
            opColor = ansi::ansi_str_bright_red();
            break;
        };

        if (true || (opScope == OperationScope::EXIT && (elapsed - lastTime) > 0.001)) {
            std::string header = format("%10.6f%9lld t%8x %-8s", elapsed, (long long)((elapsed-lastTime)*1000000000), tid, opTypeName.c_str());
            std::string indent(4 * opCount, ' ');
            std::string toDump = opColor + header + indent + opPrefix + opName
                               + ansi_str_reset() + (renderedArgs.empty() ? "" : " ") + renderedArgs + "\n";
            cerr << toDump << flush;
        }

        lastTime = startTimer.elapsed_wall();
    }
}


ScopedOperation::
~ScopedOperation()
{
    if (timer) {
        --opCount;

        if (std::uncaught_exceptions()) {
            traceOperation(OperationScope::EXIT_EXC, opType, opName);
            return;
        }

        double elapsed = timer->elapsed_wall();
        traceOperation(OperationScope::EXIT, opType, opName + " [" + ansi::ansi_str_underline() + printTime(elapsed) + "]");
    }
}

void ScopedOperation::createTimer()
{
    timer = std::make_shared<Timer>();
}

void ScopedOperation::incrementOpCount()
{
    ++opCount;
}


// GridMemoryRegionHandleInfo

std::tuple<FrozenMemoryRegion, int /* version */>
GridMemoryRegionHandleInfo::
getReadOnlyHostAccessSync(const GridComputeContext & context,
                            const std::string & opName,
                            size_t offset,
                            ssize_t length,
                            bool ignoreHazards)
{
    std::shared_ptr<const void> pin;
    if (!ignoreHazards && false) {
        pin = manager->pinAccess(opName, this, ACC_READ);
    }

    if (length == -1) {
        length = lengthInBytes - offset;
    }

    MLDB_THROW_UNIMPLEMENTED();

#if 0
    std::shared_ptr<const void> region
        = context.clQueue->clQueue.enqueueMapBufferBlocking(buffer, CL_MAP_READ,
                                                            offset /* offset */, length);

    auto done = [this, opName, ignoreHazards, region] (const void * mem)
    {
        if (!ignoreHazards) {
            unique_lock guard(mutex);
            --numReaders;
            currentReaders.erase(opName);
        }
    };

    auto pin =  std::shared_ptr<void>(nullptr, done);
    auto data = (const char *)region.get();

    return { { std::move(pin), data, lengthInBytes - offset }, this->version };
#endif
}

std::shared_ptr<const void>
GridMemoryRegionHandleInfo::
pinAccess(const std::string & opName, MemoryRegionAccess access)
{
    return manager->pinAccess(opName, this, access);
}

std::shared_ptr<const void>
GridMemoryRegionHandleInfo::Manager::
pinAccess(const std::string & opName, MemoryRegionHandleInfo * info, MemoryRegionAccess access)
{
    unique_lock guard(mutex);

    if (access == ACC_NONE) {
        throw MLDB::Exception("Asked for no access to region");
    }

    if (access == ACC_READ) {
        if (numWriters != 0 && false) {
            throw MLDB::Exception("Operation '" + opName + "' attempted to read region '" + info->name + "' with active writer(s) "
                                    + jsonEncodeStr(currentWriters) + " and active reader(s) "
                                    + jsonEncodeStr(currentReaders));
        }
        if (!currentReaders.insert(opName).second) {
            throw MLDB::Exception("Operation '" + opName + " ' is already reading region '" + info->name + "'");
        }
        ++numReaders;

        auto done = [this, opName] (const void * mem)
        {
            unique_lock guard(mutex);
            --numReaders;
            currentReaders.erase(opName);
        };

        auto pin =  std::shared_ptr<void>(nullptr, done);
        //cerr << "returning r/o version " << version << " of " << name << " for " << opName << endl;
        return pin;
    }
    else {  // read only or read write
        if (currentReaders.count(opName)) {
            throw MLDB::Exception("Operation '" + opName + " ' is already reading region '" + info->name + "'");
        }
        if (currentWriters.count(opName)) {
            throw MLDB::Exception("Operation '" + opName + " ' is already writing region '" + info->name + "'");
        }
        if ((numWriters != 0 || numReaders != 0) && false) {
            throw MLDB::Exception("Operation '" + opName + "' attempted to write region '"
                                    + info->name + "' with active writer(s) "
                                    + jsonEncodeStr(currentWriters) + " and/or active reader(s) "
                                    + jsonEncodeStr(currentReaders));
        }
        ++info->version;
        ++numWriters;
        currentWriters.insert(opName);
        if (access == ACC_READ_WRITE) {
            ++numReaders;
            currentReaders.insert(opName);
        }

        auto done = [this, info, access, opName] (const void * mem)
        {
            unique_lock guard(mutex);
            ++info->version;
            --numWriters;
            currentWriters.erase(opName);
            if (access == ACC_READ_WRITE) {
                --numReaders;
                currentReaders.erase(opName);
            }
        };

        //cerr << "returning r/w version " << version << " of " << name << " for " << opName << endl;

        //this->buffer.SetPurgeableState(mtlpp::PurgeableState::NonVolatile);

        auto pin = std::shared_ptr<void>(nullptr, done);
        return pin;
    }
}

// GridComputeProfilingInfo

GridComputeProfilingInfo::
GridComputeProfilingInfo()
{
}


// GridComputeEvent

GridComputeEvent::
GridComputeEvent(const std::string & label, bool isResolvedIn, const GridComputeQueue * owner)
    : label_(label), owner(owner), future(promise.get_future().share())
{
    ExcAssert(owner);
    if (isResolvedIn)
        resolve();
    ExcAssertEqual(this->isResolved, isResolvedIn);
}

std::shared_ptr<ComputeProfilingInfo>
GridComputeEvent::
getProfilingInfo() const
{
    MLDB_THROW_UNIMPLEMENTED();
}

void
GridComputeEvent::
resolve()
{
    if (isResolved)
        return;

    std::unique_lock guard(mutex);

    isResolved = true;
    promise.set_value();

    std::exception_ptr exc;
    for (auto & callback: callbacks) {
        try {
            callback();
        } catch (...) {
            exc = std::current_exception();
        }
    }
    if (exc)
        std::rethrow_exception(exc);
}

std::shared_ptr<ComputeEvent>
GridComputeEvent::
thenImpl(std::function<void ()> fn, const std::string & label)
{
    std::unique_lock guard(mutex);
    if (isResolved) {
        // If already satisfied, we simply run the callback
        fn();
        return owner->makeAlreadyResolvedEvent(label);
    }

#if 1
    callbacks.push_back(fn);
    return this->shared_from_this();
#else
    // Otherwise, create a user event for the post-then part
    auto nextEvent = owner->makeUnresolvedEvent(label);
    nextEvent->label_ = label + " (GridComputeEvent::thenImpl)";

    auto cb = [fn=std::move(fn), nextEvent] ()
    {
        fn();
        nextEvent->resolve();
    };

    callbacks.push_back(cb);

    return nextEvent;
#endif
}


// GridComputeQueue

GridComputeQueue::
GridComputeQueue(GridComputeContext * owner, GridComputeQueue * parent, const std::string & label,
                 GridDispatchType dispatchType)
    : ComputeQueue(owner, parent), gridOwner(owner), dispatchType(dispatchType)
{
    if (parent) {
        weakParent = parent->weak_from_this();
        ExcAssertEqual(parent->numChildren.fetch_add(1), 0);
    }
}

GridComputeQueue::~GridComputeQueue()
{
    auto sharedParent = weakParent.lock();

    if (parent && !sharedParent) {
        cerr << "ERROR: parent queue was destroyed BEFORE its children" << endl;
    }

    if (sharedParent) {
        if (sharedParent->numChildren.fetch_sub(1) == 0) {
            cerr << "ERROR: parent queue didn't know about its children" << endl;
            abort();
        }
    }

    if (numChildren != 0) {
        cerr << "ERROR: parent queue being destroyed with active children" << endl;
        abort();
    }
}

// GridComputeQueue::enqueue: in compute_kernel_grid.rs.

void
GridComputeQueue::
enqueueFillArrayImpl(const std::string & opName,
                     MemoryRegionHandle region, MemoryRegionInitialization init,
                     size_t startOffsetInBytes, ssize_t lengthInBytes,
                     std::span<const std::byte> block)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "enqueueFillArrayImpl " + opName);

    if (startOffsetInBytes > region.lengthInBytes()) {
        throw MLDB::Exception("region is too long");
    }
    if (lengthInBytes == -1)
        lengthInBytes = region.lengthInBytes() - startOffsetInBytes;
    
    if (startOffsetInBytes + lengthInBytes > region.lengthInBytes()) {
        throw MLDB::Exception("overflowing memory region");
    }

    switch (init) {
    case MemoryRegionInitialization::INIT_NONE:
        return;
    case MemoryRegionInitialization::INIT_ZERO_FILLED:
        this->enqueueZeroFillArrayConcrete(opName, region, startOffsetInBytes, lengthInBytes);
        return;
    case MemoryRegionInitialization::INIT_BLOCK_FILLED:
        this->enqueueBlockFillArrayConcrete(opName, region, startOffsetInBytes, lengthInBytes, block);
        return;
    default:
        MLDB_THROW_UNIMPLEMENTED("unknown memory region initialization %d", init);
    }
}

void
GridComputeQueue::
enqueueCopyFromHostImpl(const std::string & opName,
                        MemoryRegionHandle toRegion,
                        FrozenMemoryRegion fromRegion,
                        size_t deviceStartOffsetInBytes)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeQueue enqueueCopyFromHostImpl " + opName);
    GridMemoryRegionHandleInfo * upcastHandle = dynamic_cast<GridMemoryRegionHandleInfo *>(toRegion.handle.get());
    if (!upcastHandle) {
        throw MLDB::Exception("Wrong GridComputeContext handle: got " + demangle(typeid(toRegion.handle)));
    }
    if (upcastHandle->backingHostMem) {
        throw MLDB::Exception("cannot copy from host into a read-only host backed array");
    }

    ExcAssertLessEqual(deviceStartOffsetInBytes + fromRegion.length(), toRegion.lengthInBytes());

    this->enqueueCopyFromHostConcrete(opName, toRegion, fromRegion, deviceStartOffsetInBytes);
}

void
GridComputeQueue::
copyFromHostSyncImpl(const std::string & opName,
                            MemoryRegionHandle toRegion,
                            FrozenMemoryRegion fromRegion,
                            size_t deviceStartOffsetInBytes)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeQueue copyFromHostSyncImpl " + opName);
    enqueueCopyFromHostImpl(opName, toRegion, fromRegion, deviceStartOffsetInBytes);
    finish("copyFromHostSyncImpl");
}

FrozenMemoryRegion
GridComputeQueue::
enqueueTransferToHostImpl(const std::string & opName,
                          MemoryRegionHandle handle)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeQueue enqueueTransferToHostSyncImpl " + opName);
    return enqueueTransferToHostConcrete(opName, handle);
}

FrozenMemoryRegion
GridComputeQueue::
transferToHostSyncImpl(const std::string & opName,
                       MemoryRegionHandle handle)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeQueue transferToHostSyncImpl " + opName);

    GridMemoryRegionHandleInfo * upcastHandle = dynamic_cast<GridMemoryRegionHandleInfo *>(handle.handle.get());
    if (!upcastHandle) {
        throw MLDB::Exception("Wrong GridComputeContext handle: got " + demangle(typeid(handle)));
    }
    if (upcastHandle->backingHostMem) {
        // It's a read-only, host-first mapping... so nothing to do
        return { handle.handle, (const char *)upcastHandle->backingHostMem, (size_t)upcastHandle->lengthInBytes };
    }

    return transferToHostSyncConcrete(opName, handle);
}

MemoryRegionHandle
GridComputeQueue::
enqueueManagePinnedHostRegionImpl(const std::string & opName, std::span<const std::byte> region, size_t align,
                                  const std::type_info & type, bool isConst)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeContext managePinnedHostRegionImpl " + opName);
    return managePinnedHostRegionSyncImpl(opName, region, align, type, isConst);
}

#if 0
MemoryRegionHandle
GridComputeQueue::
managePinnedHostRegionSyncImpl(const std::string & opName,
                               std::span<const std::byte> region, size_t align,
                               const std::type_info & type, bool isConst)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeContext managePinnedHostRegionSyncImpl " + opName);

    //cerr << "managing pinned host region " << opName << " of length " << region.size() << endl;

    auto handle = std::make_shared<GridMemoryRegionHandleInfo>();
    handle->buffer = std::move(buffer);
    handle->offset = 0;
    handle->type = &type;
    handle->isConst = isConst;
    handle->lengthInBytes = region.size();
    handle->version = 0;
    handle->name = opName;
    handle->backingHostMem = region.data();
    MemoryRegionHandle result{std::move(handle)};
    return result;
}

void
GridComputeQueue::
enqueueCopyBetweenDeviceRegionsImpl(const std::string & opName,
                                    MemoryRegionHandle from, MemoryRegionHandle to,
                                    size_t fromOffset, size_t toOffset,
                                    size_t length)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeQueue enqueueCopyBetweenDeviceRegionsImpl " + opName);

    GridMemoryRegionHandleInfo * upcastFromHandle = dynamic_cast<GridMemoryRegionHandleInfo *>(from.handle.get());
    if (!upcastFromHandle) {
        throw MLDB::Exception("Wrong GridComputeContext from handle: got " + demangle(typeid(from.handle)));
    }

    auto [fromPin, fromBuffer, fromBaseOffset] = GridComputeContext::getMemoryRegion(opName, *from.handle, ACC_READ);

    ExcAssertLessEqual(fromBaseOffset + fromOffset + length, from.lengthInBytes());

    GridMemoryRegionHandleInfo * upcastToHandle = dynamic_cast<GridMemoryRegionHandleInfo *>(to.handle.get());
    if (!upcastToHandle) {
        throw MLDB::Exception("Wrong GridComputeContext to handle: got " + demangle(typeid(to.handle)));
    }

    auto [toPin, toBuffer, toBaseOffset] = GridComputeContext::getMemoryRegion(opName, *to.handle, ACC_WRITE);

    ExcAssertLessEqual(toBaseOffset + toOffset + length, to.lengthInBytes());

    auto blitEncoder = commandBuffer.BlitCommandEncoder();
    beginEncoding(opName, blitEncoder);
    blitEncoder.Copy(fromBuffer, fromBaseOffset + fromOffset, toBuffer, toBaseOffset + toOffset, length);
    endEncoding(opName, blitEncoder);
}

void
GridComputeQueue::
copyBetweenDeviceRegionsSyncImpl(const std::string & opName,
                                 MemoryRegionHandle from, MemoryRegionHandle to,
                                 size_t fromOffset, size_t toOffset,
                                 size_t length)
{
    enqueueCopyBetweenDeviceRegionsImpl(opName, from, to, fromOffset, toOffset, length);
    finish();
}
#endif

void
GridComputeQueue::
finish(const std::string & opName)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, opName + " GridComputeQueue finish");
    flush(opName)->await();
}


// GridComputeMarker

GridComputeMarker::
GridComputeMarker(const std::string & scopeName)
{
    this->scope = std::make_shared<ScopedOperation>(OperationType::USER, scopeName);
}

GridComputeMarker::
~GridComputeMarker()
{
}

std::shared_ptr<ComputeMarker>
GridComputeMarker::
enterScope(const std::string & scopeName)
{
    return std::make_shared<GridComputeMarker>(scopeName);
}


// GridComputeContext

GridComputeContext::
GridComputeContext(ComputeDevice device)
    : device(device)
{
}

ComputeDevice
GridComputeContext::
getDevice() const
{
    return device;
}

std::shared_ptr<ComputeMarker>
GridComputeContext::
getScopedMarker(const std::string & scopeName)
{
    return std::make_shared<GridComputeMarker>(scopeName);
}

void
GridComputeContext::
recordMarkerEvent(const std::string & event)
{
    traceOperation(OperationType::USER, event);
}

std::tuple<FrozenMemoryRegion, int /* version */>
GridComputeContext::
getFrozenHostMemoryRegion(const std::string & opName, MemoryRegionHandleInfo & handle,
                          size_t offset, ssize_t length,
                          bool ignoreHazards) const
{
    GridMemoryRegionHandleInfo * upcastHandle = dynamic_cast<GridMemoryRegionHandleInfo *>(&handle);
    if (!upcastHandle) {
        throw MLDB::Exception("TODO: get memory region from block handled from elsewhere: got " + demangle(typeid(handle)));
    }
    return upcastHandle->getReadOnlyHostAccessSync(*this, opName, offset, length, ignoreHazards);
}

std::shared_ptr<ComputeKernel>
GridComputeContext::
getKernel(const std::string & kernelName)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeContext getKernel " + kernelName);

    std::unique_lock guard(kernelRegistryMutex);
    auto it = kernelRegistry.find(kernelName);
    if (it == kernelRegistry.end()) {
        throw AnnotatedException(400, "Unable to find Grid compute kernel '" + kernelName + "'",
                                        "kernelName", kernelName);
    }

    // First, get the template
    // TODO: cache it...

    auto tmplate = it->second.generate(*this);

    // Secondly, specialize the template
    auto result = specializeKernel(*tmplate);

    if (traceSerializer) {
        result->traceSerializer = kernelsSerializer->newStructure(kernelName);
        result->runsSerializer = result->traceSerializer->newStructure("runs");
    }
    
    return result;
}

MemoryRegionHandle
GridComputeContext::
getSliceImpl(const MemoryRegionHandle & handle, const std::string & regionName,
             size_t startOffsetInBytes, size_t lengthInBytes,
             size_t align, const std::type_info & type, bool isConst)
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeContext getSliceImpl " + regionName);

    auto info = std::dynamic_pointer_cast<const GridMemoryRegionHandleInfo>(std::move(handle.handle));
    ExcAssert(info);

    if (info->isConst && !isConst) {
        throw MLDB::Exception("getSliceImpl: attempt to take a non-const slice of a const region");
    }

    if (*info->type != type) {
        throw MLDB::Exception("getSliceImpl: attempt to cast a slice");
    }

    if (startOffsetInBytes % align != 0) {
        throw MLDB::Exception("getSliceImpl: unaligned start offset");
    }

    if (lengthInBytes % align != 0) {
        throw MLDB::Exception("getSliceImpl: unaligned length");
    }

    if (startOffsetInBytes > info->lengthInBytes) {
        throw MLDB::Exception("getSliceImpl: start offset past the end");
    }

    if (startOffsetInBytes + lengthInBytes > info->lengthInBytes) {
        throw MLDB::Exception("getSliceImpl: end offset past the end");
    }

    std::shared_ptr<GridMemoryRegionHandleInfo> newInfo(info->clone());

    newInfo->offset = info->offset + startOffsetInBytes;
    newInfo->isConst = isConst;
    newInfo->type = &type;
    newInfo->name = regionName;
    newInfo->lengthInBytes = lengthInBytes;
    newInfo->parent = info;
    newInfo->ownerOffset = startOffsetInBytes;
    newInfo->version = info->version;

    return { newInfo };
}


// GridComputeKernel

// GridComputeKernel::allowGridPadding, allowGridExpansion, setGridExpression, addTuneable: in compute_kernel_grid.rs.

std::shared_ptr<GridBindInfo>
GridComputeKernel::
getGridBindInfo() const
{
    return std::make_shared<GridBindInfo>(this);
}


// GridBindFieldAction

DEFINE_ENUM_DESCRIPTION_INLINE(GridBindFieldActionType)
{
    addValue("SET_FIELD_FROM_PARAM", GridBindFieldActionType::SET_FIELD_FROM_PARAM);
    addValue("SET_FIELD_FROM_KNOWN", GridBindFieldActionType::SET_FIELD_FROM_KNOWN);
}

// GridBindFieldAction, GridBindAction (and their applications) and GridComputeKernelSpecialization's
// constructor: in compute_kernel_grid.rs.

BoundComputeKernel
GridComputeKernelSpecialization::
bindImpl(ComputeQueue & queue,
         std::vector<ComputeKernelArgument> argumentsIn,
         ComputeKernelConstraintSolution knowns) const
{
    auto op = scopedOperation(OperationType::GRID_COMPUTE, "GridComputeKernel bindImpl " + kernelName);

    ExcAssert(this->context);
    std::shared_ptr<GridBindInfo> bindInfo;
    {
        auto op = scopedOperation(OperationType::GRID_COMPUTE, "setup bindInfo");
        bindInfo = this->getGridBindInfo();
        bindInfo->owner = this;
    }

    //cerr << "kernel " << kernelName << " has "
    //     << computePipelineState.GetMaxTotalThreadsPerThreadgroup() << " max threads per thread group, "
    //     << computePipelineState.GetThreadExecutionWidth() << " thread execution width and requires "
    //     << computePipelineState.GetStaticThreadgroupMemoryLength() << " bytes of threadgroup memory"
    //     << endl;
#if 0
    if (computePipelineState.GetMaxTotalThreadsPerThreadgroup() < 1024) {
        cerr << "kernel " << kernelName << " has maximum execution width of "
             << computePipelineState.GetMaxTotalThreadsPerThreadgroup()
             << " meaning it may be occupancy limited due to register pressure"
             << endl;
    }
#endif

    if (traceSerializer) {
        int callNumber = numCalls++;
        bindInfo->traceSerializer = runsSerializer->newStructure(callNumber);

        if (callNumber == 0) {
            auto key = this->gridLibrary->getId();
            traceSerializer->newObject("program", key);
            traceSerializer->newObject("kernel", this->kernelName);

            // Store the program metadata so that we can replay it later
            std::unique_lock guard(tracedProgramMutex);
            if (tracedPrograms.insert(key).second) {
                auto entry = programsSerializer->newStructure(key);
                entry->newObject("metadata", this->gridLibrary->getMetadata());
            }
        }
    }

    BoundComputeKernel result;
    {
        auto op = scopedOperation(OperationType::GRID_COMPUTE, "construct result");
        result.arguments = std::move(argumentsIn);
        result.owner = this;
        result.bindInfo = bindInfo;

        // Copy constraints over
        result.constraints = this->constraints;
        result.preConstraints = this->preConstraints;
        result.postConstraints = this->postConstraints;
        result.tuneables = this->tuneables;
        result.knowns = std::move(knowns);

        result.setKnownsFromArguments();
    }

    {
        auto op = scopedOperation(OperationType::GRID_COMPUTE, "add constraints");
        // Run the setters to set the other parameters
        //for (auto & s: this->setters) {
        //    s(kernel, upcastContext);
        //}

        // Look through for constraints from dimensions
        for (size_t i = 0;  i < this->dims.size();  ++i) {
            result.addConstraint(dims[i].range, "==", "grid[" + std::to_string(i) + "]",
                                "Constraint implied by variables named in dimension " + std::to_string(0));
        }

        // Look through for constraints from parameters
        for (auto & p: this->params) {
            if (!p.type.dims.empty() && p.type.dims[0].bound) {
                result.addConstraint(p.type.dims[0].bound, p.type.dims[0].tight ? "==" : "<=", p.name + ".length",
                                    "Constraint implied by array bounds of parameter " + p.name + ": "
                                    + p.type.print());
            }
        }
    }

    // figure out the values of the new constraints
    {
        auto op = scopedOperation(OperationType::GRID_COMPUTE, "solve preConstraints");
        result.knowns = solve(result.knowns, result.constraints, result.preConstraints);
    }

    if (bindInfo->traceSerializer) {
        //bindInfo->traceSerializer->newObject("args", argInfo);
        bindInfo->traceSerializer->newObject("knowns", result.knowns);
        bindInfo->traceSerializer->newObject("constraints", result.constraints);
        bindInfo->traceSerializer->newObject("tuneables", tuneables);
    }

#if 0
    cerr << "got " << result.constraints.size() << " constraints" << endl;
    for (auto & c: result.constraints) {
        cerr << "  " << c.print()
             << (c.satisfied(result.knowns) ? " [SATSIFIED]" : " [UNSATISFIED]") << endl;
    }
#endif

    return result;
}

// GridComputeKernelTemplate::setComputeFunction: in compute_kernel_grid.rs.

std::shared_ptr<GridBindInfo>
GridComputeKernelTemplate::
getGridBindInfo() const
{
    MLDB_THROW_UNIMPLEMENTED();
}

BoundComputeKernel
GridComputeKernelTemplate::
bindImpl(ComputeQueue & queue,
         std::vector<ComputeKernelArgument> arguments,
         ComputeKernelConstraintSolution knowns) const
{
    MLDB_THROW_UNIMPLEMENTED();
}


void registerGridComputeKernel(const std::string & kernelName,
                               std::function<std::shared_ptr<GridComputeKernelTemplate>(GridComputeContext & context)> generator)
{
    kernelRegistry[kernelName].generate = generator;
}

namespace {

static struct Init {
    Init()
    {
        auto createBlockFillArrayKernel = [] (GridComputeContext& context) -> std::shared_ptr<GridComputeKernelTemplate>
        {
            auto result = std::make_shared<GridComputeKernelTemplate>(&context);
            result->kernelName = "__blockFillArray";
            result->allowGridExpansion();
            result->addParameter("region", "w", "u8[regionLength]");
            result->addParameter("startOffsetInBytes", "r", "u64");
            result->addParameter("lengthInBytes", "r", "u64");
            result->addParameter("blockData", "r", "u8[blockLengthInBytes]");
            result->addParameter("blockLengthInBytes", "r", "u64");
            result->addConstraint("blockLengthInBytes", ">", "0", "Avoid divide by zero");
            result->addConstraint("(lengthInBytes-startOffsetInBytes) % blockLengthInBytes", "==", "0", "Block must divide evenly in region");
            result->addConstraint("numBlocks", "==", "(lengthInBytes-startOffsetInBytes)/blockLengthInBytes");
            result->addTuneable("threadsPerBlock", 256);
            result->addTuneable("blocksPerGrid", 16);
            result->allowGridPadding();
            result->setGridExpression("[min(blocksPerGrid,ceilDiv(numBlocks, threadsPerBlock))]", "[threadsPerBlock]");
            result->setComputeFunction("base_kernels", "__blockFillArrayKernel");

            return result;
        };

        registerGridComputeKernel("__blockFillArray", createBlockFillArrayKernel);

        auto createZeroFillArrayKernel = [] (GridComputeContext& context) -> std::shared_ptr<GridComputeKernelTemplate>
        {
            auto result = std::make_shared<GridComputeKernelTemplate>(&context);
            result->kernelName = "__zeroFillArray";
            result->allowGridExpansion();
            result->addParameter("region", "w", "u8[regionLength]");
            result->addParameter("startOffsetInBytes", "r", "u64");
            result->addParameter("lengthInBytes", "r", "u64");
            result->addTuneable("threadsPerBlock", 256);
            result->addTuneable("blocksPerGrid", 16);
            result->allowGridPadding();
            result->setGridExpression("[blocksPerGrid]", "[threadsPerBlock]");
            result->setComputeFunction("base_kernels", "__zeroFillArrayKernel");

            return result;
        };

        registerGridComputeKernel("__zeroFillArray", createZeroFillArrayKernel);
    }

} init;

} // file scope

} // namespace MLDB
