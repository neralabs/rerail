/** compute_kernel_metal.cc                                                -*- C++ -*-
    Jeremy Barnes, 27 March 2016
    This file is part of MLDB. Copyright 2016 mldb.ai inc. All rights reserved.

    Compute kernel runtime for CPU devices.
*/

#include "compute_kernel_metal.h"
#include "mldb/types/basic_value_descriptions.h"
#include "mldb/types/meta_value_description.h"
#include "mldb/types/structure_description.h"
#include "mldb/types/set_description.h"
#include "mldb/types/generic_array_description.h"
#include "mldb/types/generic_atom_description.h"
#include "mldb/types/exception_description.h"
#include "mldb/vfs/filter_streams.h"
#include "mldb/utils/environment.h"
#include "mldb/arch/ansi.h"
#include "mldb/block/zip_serializer.h"
#include "mldb/utils/command_expression_impl.h"
#include "mldb/arch/spinlock.h"
#include "mldb/arch/exception.h"
#include "mldb/arch/timers.h"
#include "mldb/utils/string_functions.h"
#include <compare>
#include <regex>

using namespace std;

namespace MLDB {

struct MetalException: public AnnotatedException {
    static std::string getDomain(const ns::Error & error)
    {
        if (!error) return "<<no error>>";
        return error.GetDomain().GetCStr();
    }

    static std::string getDescription(const ns::Error & error)
    {
        if (!error) return "<<no error>>";
        return error.GetLocalizedDescription().GetCStr();
    }

    template<typename... NamesAndParams>
    MetalException(const ns::Error & error, const std::string & message, NamesAndParams&&... namesAndParams)
        : AnnotatedException(500, "METAL exception: " + message,
                             "domain", getDomain(error),
                             "description", getDescription(error),
                             std::forward<NamesAndParams>(namesAndParams)...),
          error(error), message(message)
    {
    }

    ns::Error error;
    std::string message;
};

template<typename... NamesAndParams>
void checkMetalError(const ns::Error & error, const std::string & message, NamesAndParams&&... namesAndParams)
{
    if (!error)
        return;

    throw MetalException(error, message, std::forward<NamesAndParams>(namesAndParams)...);
}

DEFINE_ENUM_DESCRIPTION_NAMED(MtlCommandBufferStatusDescription, mtlpp::CommandBufferStatus);

MtlCommandBufferStatusDescription::MtlCommandBufferStatusDescription()
{
    addValue("NotEnqueued", mtlpp::CommandBufferStatus::NotEnqueued, "");
    addValue("Enqueued", mtlpp::CommandBufferStatus::Enqueued, "");
    addValue("Scheduled", mtlpp::CommandBufferStatus::Scheduled, "");
    addValue("Committed", mtlpp::CommandBufferStatus::Committed, "");
    addValue("Error", mtlpp::CommandBufferStatus::Error, "");
    addValue("Completed", mtlpp::CommandBufferStatus::Completed, "");
}

namespace {

std::mutex libraryRegistryMutex;
struct LibraryRegistryEntry {
    std::function<std::shared_ptr<MetalComputeFunctionLibrary>(MetalComputeContext & context)> generate;
};

std::map<std::string, LibraryRegistryEntry> libraryRegistry;

EnvOption<int> METAL_TRACE_API_CALLS("METAL_COMPUTE_TRACE_API_CALLS", 0);
EnvOption<std::string, true> METAL_KERNEL_TRACE_FILE("METAL_KERNEL_TRACE_FILE", "");
EnvOption<std::string, true> METAL_CAPTURE_FILE("METAL_CAPTURE_FILE", "");
EnvOption<bool, false> METAL_ENABLED("METAL_ENABLED", true);

// Use the Metal capture facility to create a file that xcode can open
struct InitCapture {
    mtlpp::CaptureManager captureManager;

    InitCapture()
    {
        mtlpp::CaptureManager::GetShared().StopCapture();
    }

    void startCapture(const mtlpp::CommandQueue & commandQueue)
    {
        if (captureManager)
            return;
        
        ExcAssert(commandQueue);

        if (METAL_CAPTURE_FILE.specified()) {
            captureManager = mtlpp::CaptureManager::GetShared();
            mtlpp::CaptureDescriptor captureDescriptor;
            //captureDescriptor.SetCaptureDevice(commandQueue.GetDevice());
            if (METAL_CAPTURE_FILE.get() == "xcode") {
                //captureDescriptor.SetDestination(mtlpp::CaptureDestination::DeveloperTools);
            }
            else {
                captureDescriptor.SetDestination(mtlpp::CaptureDestination::TraceDocument);
                captureDescriptor.SetOutputURL(METAL_CAPTURE_FILE.get().c_str());
            }

            ns::Error error{ns::Handle()};
            captureManager.StartCapture(commandQueue.GetDevice());

//            if (!captureManager.StartCapture(captureDescriptor, &error)) {
            if (!captureManager.IsCapturing()) {
                throw MetalException(error, "Error initializing capture (maybe you need to set METAL_DEVICE_WRAPPER_TYPE=1 in the environment?)");
            }
        }
    }

    ~InitCapture()
    {
        if (captureManager) {
            captureManager.StopCapture();
            char buf[PATH_MAX + 1024];
            std::string fullPath = METAL_CAPTURE_FILE.get();
            if (!fullPath.empty() && fullPath[0] != '/')
                fullPath = getcwd(buf, PATH_MAX + 1024) + string("/") + METAL_CAPTURE_FILE.get();
            cerr << "Metal capture file (should have a .gputrace extension) can be inspected with the following command:" << endl;
            cerr << "osascript -e '" << endl;
            cerr << "  tell application \"Xcode\"\n    open \"" << fullPath << "\"\n  end tell" << endl;
            cerr << "'";
        }
    }
} initCapture;

struct MetalMemoryRegionHandleInfo: public GridMemoryRegionHandleInfo {
    mtlpp::Buffer buffer;

    virtual ~MetalMemoryRegionHandleInfo()
    {
        cerr << "destroying MetalMemoryRegionHandleInfo " << name << " with buffer use count " << buffer.GetRetainCount() << endl;
    }

    virtual MetalMemoryRegionHandleInfo * clone() const
    {
        return new MetalMemoryRegionHandleInfo(*this);
    }

    void init(mtlpp::Buffer mem, size_t offset)
    {
        this->buffer = std::move(mem);
        this->offset = offset;
        this->version = 0;
    }
};

} // file scope


// MetalComputeEvent

MetalComputeEvent::
MetalComputeEvent(const std::string & label, bool isResolvedIn, const MetalComputeQueue * queue)
    : GridComputeEvent(label, isResolvedIn, queue)
{
}

void
MetalComputeEvent::
resolveFromCommandBuffer(mtlpp::CommandBuffer & buffer)
{
    std::unique_lock guard{mutex};

    ExcAssert(!isResolved);
    ExcAssert(!this->commandBuffer);

    if (buffer.GetStatus() == mtlpp::CommandBufferStatus::Completed) {
        this->resolve();
        return;
    }

    if (buffer.GetStatus() == mtlpp::CommandBufferStatus::Error) {
        throw MLDB::Exception("Can't resolve command buffer with an error (TODO)");
        this->resolve();
        return;
    }

    if (buffer.GetStatus() >= mtlpp::CommandBufferStatus::Committed) {
        // https://developer.apple.com/documentation/metal/mtlcommandbuffer/1442997-addcompletedhandler?language=objc
        // You can’t add a completion handler after you commit the command buffer.
        cerr << "buffer.GetStatus() = " << jsonEncodeStr(buffer.GetStatus()) << endl;
        throw MLDB::Exception("cannot create event for committed command buffer");
    }

    std::weak_ptr<GridComputeEvent> weakThis = this->weak_from_this();

    auto onComplete = [weakThis] (const mtlpp::CommandBuffer & buffer)
    {
        auto sharedThis = std::reinterpret_pointer_cast<MetalComputeEvent>(weakThis.lock());
        if (!sharedThis)
            return;

        std::unique_lock guard(sharedThis->mutex);
        cerr << ansi::red << "onComplete for event " << sharedThis->label() << ansi::reset << endl;
        sharedThis->resolve();
        sharedThis->commandBuffer = {};
    };

    //cerr << ansi::red << "event from command buffer: status = " << (int)commandBuffer.GetStatus() << ansi::reset << endl;
    this->commandBuffer = buffer;
    commandBuffer.AddCompletedHandler(onComplete);
}

void
MetalComputeEvent::
await() const
{
    //cerr << "await(): isResolved = " << isResolved << " commandBuffer = " << commandBuffer << endl;
    if (isResolved)
        return;

    std::unique_lock guard{mutex};

    // If we're actively waiting, we can do so in this thread and avoid having to wait for
    // the asynchronous mechanisms to eventually notice that it's done.
    if (commandBuffer) {
        auto tr = scopedOperation(OperationType::METAL_COMPUTE, "awaiting " + label() + " on command buffer");
        auto mutableCommandBuffer = const_cast<mtlpp::CommandBuffer &>(commandBuffer);
        guard.unlock();

        //cerr << "await(): status = " << jsonEncodeStr(commandBuffer.GetStatus()) << endl;

        // Try a busy-ish wait
        //while (mutableCommandBuffer.GetStatus() < mtlpp::CommandBufferStatus::Completed) {
        //    std::this_thread::sleep_for(std::chrono::milliseconds(1));
        //    std::this_thread::yield();
        //}

        mutableCommandBuffer.WaitUntilCompleted();


        guard.lock();
        //cerr << "await(): status = " << jsonEncodeStr(commandBuffer.GetStatus()) << endl;

        if (!isResolved)
            ((MetalComputeEvent *)this)->resolve();
    }
    else {
        guard.unlock();
        auto tr = scopedOperation(OperationType::METAL_COMPUTE, "awaiting " + label() + " on future");        
        future.get();
        ExcAssert(isResolved);
    }
}

std::shared_ptr<MetalComputeEvent>
MetalComputeEvent::
makeAlreadyResolvedEvent(const std::string & label, const MetalComputeQueue * queue)
{
    auto result = std::make_shared<MetalComputeEvent>(label, true /* already resolved */, queue);
    return result;
}

std::shared_ptr<MetalComputeEvent>
MetalComputeEvent::
makeUnresolvedEvent(const std::string & label, const MetalComputeQueue * queue)
{
    auto result = std::make_shared<MetalComputeEvent>(label, false /* already resolved */, queue);
    return result;
}

// MetalComputeQueue

GridDispatchType generalizeDispatchType(mtlpp::DispatchType dispatchType)
{
    return dispatchType == mtlpp::DispatchType::Concurrent ? GridDispatchType::PARALLEL : GridDispatchType::SERIAL;
}

MetalComputeQueue::
MetalComputeQueue(MetalComputeContext * owner, MetalComputeQueue * parent,
                  const std::string & label,
                  mtlpp::CommandQueue queueIn, mtlpp::DispatchType dispatchType)
    : GridComputeQueue(owner, parent, label, generalizeDispatchType(dispatchType)), mtlOwner(owner),
      mtlQueue(std::move(queueIn)),
      dispatchType(dispatchType)
{
    if (!mtlQueue) {
        mtlQueue = owner->mtlDevice.NewCommandQueue(32 /* max command buffer count */);
        ExcAssert(mtlQueue);
        mtlQueue.SetLabel(ns::String(label.c_str()));
        initCapture.startCapture(mtlQueue);
    }

    commandBuffer = mtlQueue.CommandBuffer();
    ExcAssert(commandBuffer);
    commandBuffer.SetLabel(label.c_str());
}

MetalComputeQueue::~MetalComputeQueue()
{
}

std::shared_ptr<ComputeQueue>
MetalComputeQueue::
parallel(const std::string & opName)
{
    return std::make_shared<MetalComputeQueue>(this->mtlOwner, this, opName, this->mtlQueue, mtlpp::DispatchType::Concurrent);
}

std::shared_ptr<ComputeQueue>
MetalComputeQueue::
serial(const std::string & opName)
{
    return std::make_shared<MetalComputeQueue>(this->mtlOwner, this, opName, this->mtlQueue, mtlpp::DispatchType::Serial);
}

// MetalComputeQueue::enqueueZeroFillArrayConcrete, enqueueBlockFillArrayConcrete and enqueueCopyFromHostConcrete:
// in compute_kernel_metal.rs.

FrozenMemoryRegion
MetalComputeQueue::
enqueueTransferToHostConcrete(const std::string & opName, MemoryRegionHandle handle)
{
    MLDB_THROW_UNIMPLEMENTED();
}


FrozenMemoryRegion
MetalComputeQueue::
transferToHostSyncConcrete(const std::string & opName, MemoryRegionHandle handle)
{
    MLDB_THROW_UNIMPLEMENTED();
}

// MetalBindContext and MetalComputeQueue::newBindContext: in compute_kernel_metal.rs.

template<typename CommandEncoder>
void
MetalComputeQueue::
beginEncodingImpl(const std::string & opName, CommandEncoder & encoder, bool force)
{
    //cerr << "BEGIN ENCODING " << this << " " << opName << " " << type_name<CommandEncoder>() << endl;

    if (numChildren > 0) {
        throw MLDB::Exception("Cannot use a MetalComputeQueue that has a child queue once it has been created");
    }

    ExcAssert(encoder);
    encoder.SetLabel(opName.c_str());
    if (force || this->dispatchType == mtlpp::DispatchType::Serial) {
        //cerr << "awaiting " << activeCommands.size() << " fences before " << opName << endl;
        for (auto fence: activeCommands)
            encoder.WaitForFence(fence);
        activeCommands.clear();
    }
}

template<typename CommandEncoder>
void
MetalComputeQueue::
endEncodingImpl(const std::string & opName, CommandEncoder & encoder, bool force)
{
    //cerr << "END ENCODING " << this << " " << opName << " " << type_name<CommandEncoder>() << endl;
    auto fence = mtlOwner->mtlDevice.NewFence();
    ExcAssert(fence);
    encoder.UpdateFence(fence);
    encoder.EndEncoding();
    activeCommands.emplace_back(std::move(fence));
}

void
MetalComputeQueue::
beginEncoding(const std::string & opName, mtlpp::ComputeCommandEncoder & encoder, bool force)
{
    beginEncodingImpl(opName, encoder, force);
}

void
MetalComputeQueue::
beginEncoding(const std::string & opName, mtlpp::BlitCommandEncoder & encoder, bool force)
{
    beginEncodingImpl(opName, encoder, force);
}

void
MetalComputeQueue::
endEncoding(const std::string & opName, mtlpp::ComputeCommandEncoder & encoder, bool force)
{
    endEncodingImpl(opName, encoder, force);
}

void
MetalComputeQueue::
endEncoding(const std::string & opName, mtlpp::BlitCommandEncoder & encoder, bool force)
{
    endEncodingImpl(opName, encoder, force);
}

FrozenMemoryRegion
MetalComputeQueue::
enqueueTransferToHostImpl(const std::string & opName,
                          MemoryRegionHandle handle)
{
    auto op = scopedOperation(OperationType::METAL_COMPUTE, "MetalComputeQueue enqueueTransferToHostSyncImpl " + opName);
    MLDB_THROW_UNIMPLEMENTED();
}

FrozenMemoryRegion
MetalComputeQueue::
transferToHostSyncImpl(const std::string & opName,
                       MemoryRegionHandle handle)
{
    auto op = scopedOperation(OperationType::METAL_COMPUTE, "MetalComputeQueue transferToHostSyncImpl " + opName);

    MetalMemoryRegionHandleInfo * upcastHandle = dynamic_cast<MetalMemoryRegionHandleInfo *>(handle.handle.get());
    if (!upcastHandle) {
        throw MLDB::Exception("Wrong MetalComputeContext handle: got " + demangle(typeid(handle)));
    }
    if (upcastHandle->backingHostMem) {
        // It's a read-only, host-first mapping... so nothing to do
        return { handle.handle, (const char *)upcastHandle->backingHostMem, (size_t)upcastHandle->lengthInBytes };
    }

    auto [pin, buffer, offset] = MetalComputeContext::getMemoryRegion(opName, *handle.handle, ACC_READ);

    //cerr << "transferToHostSyncImpl " << this << ": status " << jsonEncodeStr(commandBuffer.GetStatus()) << endl;

    auto blitEncoder = commandBuffer.BlitCommandEncoder();
    beginEncoding(opName, blitEncoder);

    const void * contents;
    auto length = buffer.GetLength();
    ExcAssertLessEqual(handle.handle->lengthInBytes, length);

    switch (buffer.GetStorageMode()) {
    case mtlpp::StorageMode::Managed:
        blitEncoder.Synchronize(buffer);
        // fall through
    case mtlpp::StorageMode::Shared:
        contents = buffer.GetContents();
        break;
    case mtlpp::StorageMode::Private: {
        // It's a private region; we need to create a new region to hold the result, and then
        // synchronize
        mtlpp::ResourceOptions options = mtlpp::ResourceOptions::StorageModeManaged;
        auto tmpBuffer = mtlOwner->mtlDevice.NewBuffer(length == 0 ? 4 : length, options);
        tmpBuffer.SetLabel((opName + " transferToHostSync private copy").c_str());
        blitEncoder.Copy(buffer, offset, tmpBuffer, 0 /* offset */, length - offset);
        blitEncoder.Synchronize(tmpBuffer);
        contents = tmpBuffer.GetContents();

        // We pin the temp buffer instead
        auto freeTmpBuffer = [tmpBuffer] (auto arg) {};
        pin = std::shared_ptr<const void>(nullptr, freeTmpBuffer);
        break;
    }
    default:
        throw MLDB::Exception("Cannot manage MemoryLess storage mode");
    }
    endEncoding(opName, blitEncoder);

    auto myCommandBuffer = commandBuffer;

    finish("transferToHostSync");

    //ExcAssert(commandBuffer);
    //commandBuffer.Commit();
    //commandBuffer.WaitUntilCompleted();

    auto status = myCommandBuffer.GetStatus();
    if (status != mtlpp::CommandBufferStatus::Completed) {
        auto error = commandBuffer.GetError();
        throw MetalException(error, "Command buffer is not completed", "status", status);
    }

    //auto storageMode = buffer.GetStorageMode();
    //cerr << "storage mode is " << (int)storageMode << endl;


    //cerr << "contents " << contents << endl;
    //cerr << "length " << length << endl;

    //cerr << "region length for region " << handle.handle->name
    //     << " with lengthInBytes " << handle.handle->lengthInBytes
    //     << " is " << length << " and contents " << contents << endl;

    FrozenMemoryRegion result(std::move(pin), ((const char *)contents) + offset, handle.handle->lengthInBytes);
    return result;
}

MutableMemoryRegion
MetalComputeQueue::
enqueueTransferToHostMutableImpl(const std::string & opName, MemoryRegionHandle handle)
{
    MLDB_THROW_UNIMPLEMENTED();

#if 0
    auto op = scopedOperation(OperationType::METAL_COMPUTE, "MetalComputeContext transferToHostMutableImpl " + opName);
    ExcAssert(handle.handle);

    auto [pin, mem, offset] = getMemoryRegion(opName, *handle.handle, ACC_READ_WRITE);
    MetalEvent clEvent;
    std::shared_ptr<void> memPtr;
    std::tie(memPtr, clEvent)
        = clQueue->clQueue.enqueueMapBuffer(mem, CL_MAP_READ | CL_MAP_WRITE,
                                    offset, handle.lengthInBytes());

    auto event = std::make_shared<MetalComputeEvent>(std::move(clEvent));
    auto promise = std::shared_ptr<std::promise<std::any>>();

    auto cb = [handle, promise, memPtr, pin=pin] (const MetalEvent & event, auto status)
    {
        if (status == MetalEventCommandExecutionStatus::ERROR)
            promise->set_exception(std::make_exception_ptr(MLDB::Exception("Metal error mapping host memory")));
        else {
            promise->set_value(MutableMemoryRegion(memPtr, (char *)memPtr.get(), handle.lengthInBytes()));
        }
    };

    clEvent.addCallback(cb);

    return { std::move(promise), std::move(event) };
#endif
}

MutableMemoryRegion
MetalComputeQueue::
transferToHostMutableSyncImpl(const std::string & opName,
                              MemoryRegionHandle handle)
{
    MLDB_THROW_UNIMPLEMENTED();

#if 0
    auto op = scopedOperation(OperationType::METAL_COMPUTE, "MetalComputeContext transferToHostMutableSyncImpl " + opName);

    ExcAssert(handle.handle);

    auto [pin, mem, offset] = getMemoryRegion(opName, *handle.handle, ACC_READ_WRITE);
    auto memPtr = clQueue->clQueue.enqueueMapBufferBlocking(mem, CL_MAP_READ | CL_MAP_WRITE,
                                                            offset, handle.lengthInBytes());
    MutableMemoryRegion result(std::move(memPtr), (char *)memPtr.get(), handle.lengthInBytes());
    return result;
#endif
}

MemoryRegionHandle
MetalComputeQueue::
enqueueManagePinnedHostRegionImpl(const std::string & opName, std::span<const std::byte> region, size_t align,
                                  const std::type_info & type, bool isConst)
{
    auto op = scopedOperation(OperationType::METAL_COMPUTE, "MetalComputeContext managePinnedHostRegionImpl " + opName);
    return managePinnedHostRegionSyncImpl(opName, region, align, type, isConst);
}

MemoryRegionHandle
MetalComputeQueue::
managePinnedHostRegionSyncImpl(const std::string & opName,
                               std::span<const std::byte> region, size_t align,
                               const std::type_info & type, bool isConst)
{
    auto op = scopedOperation(OperationType::METAL_COMPUTE, "MetalComputeContext managePinnedHostRegionSyncImpl " + opName);

    //cerr << "managing pinned host region " << opName << " of length " << region.size() << endl;

    mtlpp::ResourceOptions options
         = mtlpp::ResourceOptions::StorageModeShared;


    traceMetalOperation("region size " + std::to_string(region.size()));

    // This overload calls newBufferWithBytesNoCopy
    // Note that we require a page-aligned address and size, and a single VM region
    // See https://developer.apple.com/documentation/metal/mtldevice/1433382-makebuffer
    // Thus, for now we don't try; instead we copy and blit
    mtlpp::Buffer buffer;
    if (false) {
        auto deallocator = [=] (auto ptr, auto len)
        {
            cerr << ansi::bright_red << "DEALLOCATING PINNED REGION " << opName << ansi::reset
                << " of length " << len << endl;
        };

        buffer = mtlOwner->mtlDevice.NewBuffer((void *)region.data(), region.size(), options, deallocator);
    }
    else {
        if (region.empty()) {
            buffer = mtlOwner->mtlDevice.NewBuffer(4, options);
        }
        else {
            buffer = mtlOwner->mtlDevice.NewBuffer((const void *)region.data(), region.size(), options);
        }
    }
    buffer.SetLabel(opName.c_str());

    auto handle = std::make_shared<MetalMemoryRegionHandleInfo>();
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

// MetalComputeQueue::enqueueCopyBetweenDeviceRegionsImpl: in compute_kernel_metal.rs.

void
MetalComputeQueue::
copyBetweenDeviceRegionsSyncImpl(const std::string & opName,
                                 MemoryRegionHandle from, MemoryRegionHandle to,
                                 size_t fromOffset, size_t toOffset,
                                 size_t length)
{
    enqueueCopyBetweenDeviceRegionsImpl(opName, from, to, fromOffset, toOffset, length);
    finish("copyBetweenDeviceRegionsSyncImpl");
}

std::shared_ptr<ComputeEvent>
MetalComputeQueue::
flush(const std::string & opName)
{
    auto op = scopedOperation(OperationType::METAL_COMPUTE, opName + " MetalComputeQueue flush");

    auto result = std::make_shared<MetalComputeEvent>(opName, false /* resolved */, this);
    ExcAssert(commandBuffer);

    //cerr << "flush(): " << this << " status = " << jsonEncodeStr(commandBuffer.GetStatus()) << endl;

    if (commandBuffer.GetStatus() < mtlpp::CommandBufferStatus::Completed) {
        result->resolveFromCommandBuffer(commandBuffer);
        if (commandBuffer.GetStatus() < mtlpp::CommandBufferStatus::Committed)
            commandBuffer.Commit();
    }
    else {
        // TODO: error
        result->resolve();
    }

    //cerr << "flush(): " << this << " status now " << jsonEncodeStr(commandBuffer.GetStatus()) << endl;

    auto label = commandBuffer.GetLabel();

    commandBuffer = mtlQueue.CommandBuffer();
    commandBuffer.SetLabel(label);
    ExcAssert(commandBuffer);

    //cerr << "flush(): " << this << " status at end is " << jsonEncodeStr(commandBuffer.GetStatus()) << endl;

    return result;
}

void
MetalComputeQueue::
enqueueBarrier(const std::string & label)
{
    auto op = scopedOperation(OperationType::METAL_COMPUTE, "MetalComputeQueue enqueueBarrier " + label);
    auto encoder = commandBuffer.ComputeCommandEncoder(mtlpp::DispatchType::Serial);
    beginEncoding("enqueueBarrier " + label, encoder, true /* force */);
    endEncoding("enqueueBarrier " + label, encoder, true /* force */);
}

void
MetalComputeQueue::
finish(const std::string & opName)
{
    auto op = scopedOperation(OperationType::METAL_COMPUTE, opName + " MetalComputeQueue finish");
    flush(opName)->await();
}

std::shared_ptr<ComputeEvent>
MetalComputeQueue::
makeAlreadyResolvedEvent(const std::string & label) const
{
    return MetalComputeEvent::makeAlreadyResolvedEvent(label, this);
}


// MetalComputeContext

MetalComputeContext::
MetalComputeContext(mtlpp::Device mtlDevice, ComputeDevice device)
    : GridComputeContext(device), mtlDevice(mtlDevice)
{
    {
#if 0
        mtlpp::Buffer buffer;

        for (size_t i = 0;  i < 1000;  ++i) {
            cerr << endl << "creating buffer " << i << endl;
            buffer = mtlDevice.NewBuffer(100000000, mtlpp::ResourceOptions::StorageModePrivate);
            cerr << endl << "done creating buffer " << i << endl;
            if (!buffer) {
                throw MLDB::Exception("Error creating buffer number " + std::to_string(i));
            }
            //buffer.SetLabel(format("test buffer %zd", i).c_str());
            //ns::GarbageCollectExhaustively();
        }

        cerr << "done creating buffers" << endl;
#endif
    }

#if 0
    mtlpp::HeapDescriptor heapDescriptor;
    heapDescriptor.SetStorageMode(mtlpp::StorageMode::Private);
    heapDescriptor.SetSize(1 << 30);  // 1GB

    this->heap = mtlDevice.NewHeap(heapDescriptor);
    ExcAssert(this->heap);
#endif
}


std::tuple<std::shared_ptr<const void>, mtlpp::Buffer, size_t>
MetalComputeContext::
getMemoryRegion(const std::string & opName, MemoryRegionHandleInfo & handle, MemoryRegionAccess access)
{
    MetalMemoryRegionHandleInfo * upcastHandle = dynamic_cast<MetalMemoryRegionHandleInfo *>(&handle);
    if (!upcastHandle) {
        throw MLDB::Exception("TODO: get memory region from block handled from elsewhere: got " + demangle(typeid(handle)));
    }
    auto pin = upcastHandle->pinAccess(opName, access);

    return { std::move(pin), upcastHandle->buffer, upcastHandle->offset };
}

std::tuple<FrozenMemoryRegion, int /* version */>
MetalComputeContext::
getFrozenHostMemoryRegion(const std::string & opName, MemoryRegionHandleInfo & handle,
                          size_t offset, ssize_t length,
                          bool ignoreHazards) const
{
    MetalMemoryRegionHandleInfo * upcastHandle = dynamic_cast<MetalMemoryRegionHandleInfo *>(&handle);
    if (!upcastHandle) {
        throw MLDB::Exception("TODO: get memory region from block handled from elsewhere: got " + demangle(typeid(handle)));
    }
    return upcastHandle->getReadOnlyHostAccessSync(*this, opName, offset, length, ignoreHazards);
}

static MemoryRegionHandle
doMetalAllocate(mtlpp::Heap & mtlHeap,
                mtlpp::Device & mtlDevice,
                 const std::string & regionName,
                 size_t length, size_t align,
                 const std::type_info & type,
                 bool isConst)
{
    cerr << "allocating " << length << " bytes for " << regionName << endl;
    // TODO: align...
    //mtlpp::ResourceOptions options = mtlpp::ResourceOptions::StorageModeManaged;
    mtlpp::ResourceOptions options = mtlpp::ResourceOptions::StorageModePrivate;
    auto buffer = mtlDevice.NewBuffer(length == 0 ? 4 : length, options);
    //auto buffer = mtlHeap.NewBuffer(length == 0 ? 4 : length, options);
    if (!buffer) {
        throw MLDB::Exception("Error allocating buffer from heap");
    }
    buffer.SetLabel(regionName.c_str());

    auto handle = std::make_shared<MetalMemoryRegionHandleInfo>();
    handle->buffer = std::move(buffer);
    handle->offset = 0;
    handle->type = &type;
    handle->isConst = isConst;
    handle->lengthInBytes = length;
    handle->name = regionName;
    handle->version = 0;

    MemoryRegionHandle result{std::move(handle)};
    return result;
}

MemoryRegionHandle
MetalComputeContext::
allocateSyncImpl(const std::string & regionName,
                 size_t length, size_t align,
                 const std::type_info & type, bool isConst)
{
    auto op = scopedOperation(OperationType::METAL_COMPUTE, "MetalComputeContext allocateSyncImpl " + regionName);
    auto result = doMetalAllocate(this->heap, this->mtlDevice, regionName, length, align, type, isConst);
    return result;
}

static MemoryRegionHandle
doMetalTransferToDevice(mtlpp::Device & mtlDevice,
                         const std::string & opName, FrozenMemoryRegion region,
                         const std::type_info & type, bool isConst)
{
    Timer timer;

    mtlpp::ResourceOptions options = mtlpp::ResourceOptions::StorageModeManaged;
    
    mtlpp::Buffer buffer;
    if (region.length() == 0)
        buffer = mtlDevice.NewBuffer(4, options);
    else {
        buffer = mtlDevice.NewBuffer(region.data(), region.length(), options);
    }
    buffer.SetLabel(opName.c_str());

    auto handle = std::make_shared<MetalMemoryRegionHandleInfo>();
    handle->buffer = std::move(buffer);
    handle->offset = 0;
    handle->type = &type;
    handle->isConst = isConst;
    handle->lengthInBytes = region.length();
    handle->name = opName;
    handle->version = 0;
    MemoryRegionHandle result{std::move(handle)};

    using namespace std;
    cerr << "transferring " << opName << " " << region.memusage() / 1000000.0 << " Mbytes of type "
        << demangle(type.name()) << " isConst " << isConst << " to device in "
            << timer.elapsed_wall() << " at "
            << region.memusage() / 1000000.0 / timer.elapsed_wall() << "MB/sec" << endl;

    return result;
}

std::shared_ptr<ComputeQueue>
MetalComputeContext::
getQueue(const std::string & queueName)
{
    return std::make_shared<MetalComputeQueue>(this, nullptr /* parent */, queueName, mtlpp::CommandQueue(), mtlpp::DispatchType::Serial);
}

std::shared_ptr<GridComputeFunctionLibrary>
MetalComputeContext::
getLibrary(const std::string & libraryName)
{
    auto op = scopedOperation(OperationType::METAL_COMPUTE, "MetalComputeContext getLibrary " + libraryName);

    std::unique_lock guard(libraryRegistryMutex);
    auto it = libraryRegistry.find(libraryName);
    if (it == libraryRegistry.end()) {
        throw AnnotatedException(400, "Unable to find Metal compute library '" + libraryName + "'",
                                        "libraryName", libraryName);
    }
    auto result = it->second.generate(*this);
    //if (traceSerializer) {
    //    result->traceSerializer = kernelsSerializer->newStructure(kernelName);
    //    result->runsSerializer = result->traceSerializer->newStructure("runs");
    //}
    return result;
}

std::shared_ptr<GridComputeKernelSpecialization>
MetalComputeContext::
specializeKernel(const GridComputeKernelTemplate & tmplate)
{
    return std::make_shared<MetalComputeKernel>(this, tmplate);
}


// MetalComputeKernel

// makeBasicType, getKernelTypeFromDataType, convertAccess, getKernelTypeFrom{Struct,Array}Type and
// getKernelType: in compute_kernel_metal.rs.

MetalComputeKernel::
MetalComputeKernel(MetalComputeContext * owner, const GridComputeKernelTemplate & tmplate)
    : GridComputeKernelSpecialization(owner, tmplate)
{
    this->mtlContext = owner;
    this->mtlFunction = dynamic_cast<const MetalComputeFunction *>(this->gridFunction.get());
    ExcAssert(this->mtlFunction);
}


// MetalComputeFunction

// MetalComputeFunction (pipeline state with reflection), getDisposition and getArgumentInfo: in
// compute_kernel_metal.rs.

// MetalComputeFunctionLibrary's constructor and getFunction: in compute_kernel_metal.rs.

std::string
MetalComputeFunctionLibrary::
getId() const
{
    MLDB_THROW_UNIMPLEMENTED();
}

Json::Value
MetalComputeFunctionLibrary::
getMetadata() const
{
    MLDB_THROW_UNIMPLEMENTED();
}

std::string runCommand(const std::vector<std::string> & command, const std::string & debugName)
{
    std::ostringstream preprocessedOutput;
    std::ostringstream preprocessorErrors;

    auto stdOutSink = std::make_shared<OStreamInputSink>(&preprocessedOutput);
    auto stdErrSink = std::make_shared<OStreamInputSink>(&preprocessorErrors);

    RunResult result = execute(command, stdOutSink, stdErrSink);

    switch (result.state) {
    case RunResult::LAUNCH_ERROR:
        throw MLDB::AnnotatedException(500, debugName + " launch error",
                                        "command", command,
                                        "error", result.launchError);
    case RunResult::LAUNCH_EXCEPTION:
        throw MLDB::AnnotatedException(500, debugName + " launch exception",
                                        "command", command,
                                        "exc", result.launchExc);
    case RunResult::SIGNALED:
        throw MLDB::AnnotatedException(500, debugName + " returned signal",
                                        "command", command,
                                        "signal", result.signum,
                                        "cerr", preprocessorErrors.str());
    case RunResult::RETURNED:
        if (result.returnCode != 0)
            throw MLDB::AnnotatedException(500, debugName + " returned non-zero result",
                                        "command", command,
                                           "result", result.returnCode,
                                           "cerr", MLDB::split(preprocessorErrors.str(), '\n'));
        break;
    case RunResult::PARENT_EXITED:
        throw MLDB::AnnotatedException(500, debugName + " parent exited (logic error)");
    case RunResult::UNKNOWN:
        throw MLDB::AnnotatedException(500, debugName + " state unknown (logic error)");
    }

    return preprocessedOutput.str();
}

std::shared_ptr<MetalComputeFunctionLibrary>
MetalComputeFunctionLibrary::
compileFromSourceFile(MetalComputeContext & context, const std::string & fileName)
{
    filter_istream stream(fileName);
    Utf8String source = /*"#line 1 \"" + fileName + "\"\n" +*/ stream.readAll();

    static const struct MetalQuery {
        MetalQuery()
        {
            std::string output = runCommand({"/usr/bin/env", "xcrun",
                                             "-sdk", "macosx", "metal",
                                             "-E", "mldb/builtin/metal/query_metal.metal"}, "Metal version query");

            std::regex get_version("mldb_metal_version=([0-9]+)", std::regex_constants::syntax_option_type::multiline);
            std::smatch match;
            if (!std::regex_search(output, match, get_version)) {
                throw AnnotatedException(500, "did not find metal version");
            }
            version = match[1].str();
            cerr << "Metal Query: version = " << version << endl;
        }

        std::string version;
    } metalQuery;

    // TODO: preprocessor that simply expands the __UKL macros and includes non-metal includes, rather than do the whole lot...
    std::string preprocessedOutput
         = runCommand({"/usr/bin/env", "c++", "-C", "-CC", "-xc++",
                      "-I./mldb/block/ukl",
                      "-I./mldb/builtin/metal/ukl",
                      "-D__METAL_VERSION__=" + metalQuery.version,
                      "-D__METAL_MACOS__=1",
                      "-D__UKL_METAL_PREPROCESSING=1",
                      "-E", fileName }, "Preprocessor");

    //RunResult result = execute({"/usr/bin/env", "xcrun",
    //                            "-sdk", "macosx", "metal",
    //                            "-frecord-sources",
    //                            "-I.", "-I./block/ukl", "-I./builtin/metal/ukl",
    //                            "-E", fileName },
    //                            stdOutSink, stdErrSink);


    cerr << "got preprocessed source: " << endl << preprocessedOutput << endl;

    return compileFromSource(context, preprocessedOutput, fileName);
}

// compileFromSource: in compute_kernel_metal.rs.

std::shared_ptr<MetalComputeFunctionLibrary>
MetalComputeFunctionLibrary::
loadMtllib(MetalComputeContext & context, const std::string & libraryFilename)
{
    ns::Error error{ns::Handle()};

    mtlpp::Library library = context.mtlDevice.NewLibrary(libraryFilename.c_str(), &error);

    checkMetalError(error, "Error loading Metal library", "fileName", libraryFilename);

    return std::make_shared<MetalComputeFunctionLibrary>(context, std::move(library));
}


// MetalComputeRuntime

EnvOption<int> METAL_DEFAULT_DEVICE("METAL_DEFAULT_DEVICE", -1);

struct MetalComputeRuntime: public ComputeRuntime {

    std::vector<mtlpp::Device> mtlDevices;
    std::vector<ComputeDevice> devices;

    MetalComputeRuntime()
    {
        auto queriedDevices = mtlpp::Device::CopyAllDevices();
        for (size_t i = 0;  i < queriedDevices.GetSize();  ++i) {
            auto device = queriedDevices[i];
            //cerr << "metal device " << i << " is " << device.GetName().GetCStr() << endl;
            mtlDevices.push_back(queriedDevices[i]);
            devices.push_back({ComputeRuntimeId::METAL, 0 /* runtime instance */, (uint8_t)i, 0, 0});
        }
    }

    mtlpp::Device convertDevice(ComputeDevice device) const
    {
        if (device.runtime != ComputeRuntimeId::METAL) {
            throw MLDB::Exception("Attempt to pass non-Metal device " + device.info() + " to Metal");
        }
        return mtlDevices.at(device.deviceInstance);
    }

    virtual ~MetalComputeRuntime()
    {
    }

    virtual ComputeRuntimeId getId() const
    {
        return ComputeRuntimeId::METAL;
    }

    virtual std::string printRestOfDevice(ComputeDevice device) const
    {
        return std::to_string(device.deviceInstance);
    }

    virtual std::string printHumanReadableDeviceInfo(ComputeDevice device) const
    {
        if (device.runtimeInstance > 0 || device.deviceInstance >= mtlDevices.size()) {
            return "<<INVALID METAL PLATFORM OR DEVICE INDEX>>";
        }
        std::string result = mtlDevices[device.deviceInstance].GetName().GetCStr();
        return result;
    }

    virtual ComputeDevice getDefaultDevice() const
    {
        if (!METAL_ENABLED || mtlDevices.empty())
            return ComputeDevice::none();

        if (METAL_DEFAULT_DEVICE.specified()) {
            ExcAssertLess(METAL_DEFAULT_DEVICE.get(), mtlDevices.size());
            return {ComputeRuntimeId::METAL, 0 /* platform */,
                    (uint16_t)METAL_DEFAULT_DEVICE.get(), 0, 0};
        }

        // TODO: deal with the possibility of the default not being at index zero...
        return {ComputeRuntimeId::METAL, 0 /* platform */, 0 /* device */,0, 0};
    }

    // Enumerate the devices available for this runtime
    virtual std::vector<ComputeDevice> enumerateDevices() const
    {
        return devices;
    }

    // Get a compute context for this runtime
    virtual std::shared_ptr<ComputeContext>
    getContext(std::span<const ComputeDevice> devices) const
    {
        if (devices.size() != 1) {
            throw MLDB::Exception("Metal Compute Kernel driver only can accept a single device");
        }
        return std::make_shared<MetalComputeContext>(convertDevice(devices[0]), devices[0]);
    }

};

void registerMetalLibrary(const std::string & libraryName,
                          std::function<std::shared_ptr<MetalComputeFunctionLibrary>(MetalComputeContext &)> generator)
{
    std::unique_lock guard{libraryRegistryMutex};
    libraryRegistry[libraryName].generate = generator;
}

namespace {

static struct Init {
    Init()
    {
        ComputeRuntime::registerRuntime(ComputeRuntimeId::METAL, "metal",
                                        [] () { return new MetalComputeRuntime(); });

        auto compileLibrary = [] (MetalComputeContext & context) -> std::shared_ptr<MetalComputeFunctionLibrary>
        {
            if (false) {
                std::string fileName = "mldb/builtin/metal/base_kernels.metal";
                return MetalComputeFunctionLibrary::compileFromSourceFile(context, fileName);
            }
            else {
                std::string fileName = "build/arm64/lib/base_kernels_metal.metallib";
                return MetalComputeFunctionLibrary::loadMtllib(context, fileName);
            }
        };

        registerMetalLibrary("base_kernels", compileLibrary);
    }

} init;


} // file scope
} // namespace MLDB
