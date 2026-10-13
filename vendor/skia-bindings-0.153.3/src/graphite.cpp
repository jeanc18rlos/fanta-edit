#include "bindings.h"

#include <algorithm>
#include <atomic>
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <list>
#include <memory>
#include <unordered_map>

#include "include/core/SkCanvas.h"
#include "include/core/SkColorSpace.h"
#include "include/core/SkContext.h"
#include "include/core/SkImage.h"
#include "include/core/SkRect.h"
#include "include/core/SkSize.h"
#include "include/core/SkSurface.h"
#include "include/gpu/GpuTypes.h"
#include "include/gpu/graphite/BackendTexture.h"
#include "include/gpu/graphite/Context.h"
#include "include/gpu/graphite/ContextOptions.h"
#include "include/gpu/graphite/GraphiteTypes.h"
#include "include/gpu/graphite/Image.h"
#include "include/gpu/graphite/ImageProvider.h"
#include "include/gpu/graphite/Recorder.h"
#include "include/gpu/graphite/Surface.h"
#include "include/gpu/graphite/TextureInfo.h"

#ifdef SK_METAL
#include "include/gpu/graphite/mtl/MtlBackendContext.h"
#include "include/gpu/graphite/mtl/MtlGraphiteTypes_cpp.h"
#endif

#ifdef SK_VULKAN
#include "include/gpu/vk/VulkanBackendContext.h"
#include "include/gpu/graphite/vk/VulkanGraphiteContext.h"
#endif

// Forward declaration to avoid including Recording.h which exposes std::unordered_set
namespace skgpu::graphite {
    class Recording;
}

extern "C" void C_GraphiteUnreferencedTypes(skgpu::Budgeted *, skgpu::Mipmapped *, skgpu::Budgeted *) {}

//
// gpu/graphite/BackendTexture.h
//

extern "C" void C_BackendTexture_Construct(skgpu::graphite::BackendTexture* uninitialized) {
    new(uninitialized) skgpu::graphite::BackendTexture();
}

extern "C" void C_BackendTexture_CopyConstruct(skgpu::graphite::BackendTexture* uninitialized, const skgpu::graphite::BackendTexture* backendTexture) {
    new(uninitialized) skgpu::graphite::BackendTexture(*backendTexture);
}

extern "C" void C_BackendTexture_destruct(skgpu::graphite::BackendTexture* self) {
    self->~BackendTexture();
}

extern "C" bool C_BackendTexture_isValid(const skgpu::graphite::BackendTexture* self) {
    return self->isValid();
}

extern "C" skgpu::BackendApi C_BackendTexture_backend(const skgpu::graphite::BackendTexture* self) {
    return self->backend();
}

extern "C" void C_BackendTexture_dimensions(const skgpu::graphite::BackendTexture* self, SkISize* dimensions) {
    *dimensions = self->dimensions();
}

extern "C" void C_BackendTexture_info(const skgpu::graphite::BackendTexture* self, skgpu::graphite::TextureInfo* info) {
    *info = self->info();
}

//
// gpu/graphite/TextureInfo.h
//

extern "C" void C_TextureInfo_Construct(skgpu::graphite::TextureInfo* uninitialized) {
    new(uninitialized) skgpu::graphite::TextureInfo();
}

extern "C" void C_TextureInfo_destruct(skgpu::graphite::TextureInfo* self) {
    self->~TextureInfo();
}

extern "C" bool C_TextureInfo_isValid(const skgpu::graphite::TextureInfo* self) {
    return self->isValid();
}

extern "C" skgpu::BackendApi C_TextureInfo_backend(const skgpu::graphite::TextureInfo* self) {
    return self->backend();
}

extern "C" bool C_TextureInfo_Equals(const skgpu::graphite::TextureInfo* lhs, const skgpu::graphite::TextureInfo* rhs) {
    return *lhs == *rhs;
}

//
// gpu/graphite/Context.h
//

namespace {
class FantaImageProvider final : public skgpu::graphite::ImageProvider {
public:
    sk_sp<SkImage> findOrCreate(skgpu::graphite::Recorder* recorder,
                               const SkImage* image,
                               SkImage::RequiredProperties properties) override {
        if (fConversionFailed) {
            return nullptr;
        }
        const uint64_t key = (static_cast<uint64_t>(image->uniqueID()) << 1) |
                             static_cast<uint64_t>(properties.fMipmapped);
        auto found = fImages.find(key);
        if (found != fImages.end()) {
            fRecent.splice(fRecent.begin(), fRecent, found->second);
            return found->second->fImage;
        }
        auto texture = SkImages::TextureFromImage(recorder, image, properties);
        if (!texture) {
            fConversionFailed = true;
            return nullptr;
        }
        const size_t bytes = texture->textureSize();
        if (bytes == 0 || bytes > kMaximumBytes) {
            return texture;
        }
        while (fRecent.size() >= kMaximumImages || fBytes > kMaximumBytes - bytes) {
            auto& oldest = fRecent.back();
            fBytes -= oldest.fBytes;
            fImages.erase(oldest.fKey);
            fRecent.pop_back();
        }
        fRecent.push_front({key, texture, bytes});
        fImages.emplace(key, fRecent.begin());
        fBytes += bytes;
        return texture;
    }

    bool conversionFailed() const { return fConversionFailed; }

    void discardRecording() {
        // Cached textures may depend on uploads in the discarded recording.
        fImages.clear();
        fRecent.clear();
        fBytes = 0;
        fConversionFailed = false;
    }

private:
    struct Entry {
        uint64_t fKey;
        sk_sp<SkImage> fImage;
        size_t fBytes;
    };
    static constexpr size_t kMaximumImages = 512;
    static constexpr size_t kMaximumBytes = 512 * (size_t{1} << 20);
    std::list<Entry> fRecent;
    std::unordered_map<uint64_t, std::list<Entry>::iterator> fImages;
    size_t fBytes = 0;
    bool fConversionFailed = false;
};

// Recorders and their child objects are single-threaded. The recorder owns the
// provider; this non-owning registry only identifies our provider at snap/drop.
thread_local std::unordered_map<skgpu::graphite::Recorder*, FantaImageProvider*> fantaProviders;
}  // namespace

extern "C" skgpu::graphite::Recorder* C_Context_makeRecorder(skgpu::graphite::Context* self, const skgpu::graphite::RecorderOptions* options) {
    skgpu::graphite::RecorderOptions configured(*options);
    if (configured.fGpuBudgetInBytes == skgpu::graphite::RecorderOptions::kDefaultRecorderBudget) {
        configured.fGpuBudgetInBytes = 1024 * (size_t{1} << 20);
    }
    sk_sp<FantaImageProvider> provider;
    if (!configured.fImageProvider) {
        provider = sk_make_sp<FantaImageProvider>();
        configured.fImageProvider = provider;
    }
    auto recorder = self->makeRecorder(configured);
    if (recorder && provider) {
        fantaProviders.emplace(recorder.get(), provider.get());
    }
    return recorder.release();
}

extern "C" skgpu::graphite::InsertStatus::V C_Context_insertRecording(skgpu::graphite::Context* self, const skgpu::graphite::InsertRecordingInfo* info) {
    return self->insertRecording(*info);
}

extern "C" bool C_Context_submit(skgpu::graphite::Context* self, const skgpu::graphite::SubmitInfo* submitInfo) {
    return self->submit(submitInfo ? *submitInfo : skgpu::graphite::SubmitInfo{});
}

extern "C" void C_Context_checkAsyncWorkCompletion(skgpu::graphite::Context* self) {
    self->checkAsyncWorkCompletion();
}

extern "C" void C_Context_deleteBackendTexture(skgpu::graphite::Context* self, const skgpu::graphite::BackendTexture* backendTexture) {
    self->deleteBackendTexture(*backendTexture);
}

extern "C" bool C_Context_isDeviceLost(const skgpu::graphite::Context* self) {
    return self->isDeviceLost();
}

// skgpu::graphite::Context is owned via std::unique_ptr (Context::MakeMetal etc.
// return unique_ptr and the shim releases it). It is NOT ref-counted, so the
// Rust wrapper must `delete` it rather than unref a (non-existent) SkRefCntBase.
extern "C" void C_Context_delete(skgpu::graphite::Context* self) {
    delete self;
}

// Synchronous pixel readback for a Graphite-backed surface.
//
// Graphite is a deferred backend: SkSurface::readPixels() does not work, so the
// only supported readback is Context::asyncRescaleAndReadPixels() driven to
// completion by a synchronous submit. This shim hides the async callback dance
// behind a blocking call that copies one plane into the caller's buffer, which
// is what a screenshot / golden-image path needs.
namespace {
struct SyncReadContext {
    std::atomic<bool> fCalled{false};
    std::unique_ptr<const SkImage::AsyncReadResult> fResult;
};

void sync_read_callback(
    SkImage::ReadPixelsContext context,
    std::unique_ptr<const SkImage::AsyncReadResult> result) {
    std::unique_ptr<std::shared_ptr<SyncReadContext>> owner(
        static_cast<std::shared_ptr<SyncReadContext>*>(context));
    (*owner)->fResult = std::move(result);
    (*owner)->fCalled.store(true, std::memory_order_release);
}
}  // namespace

extern "C" bool C_Context_readPixels(
    skgpu::graphite::Context* self,
    SkSurface* surface,
    const SkImageInfo* dstInfo,
    void* dstPixels,
    size_t dstRowBytes,
    int srcX,
    int srcY) {
    const size_t minRowBytes = dstInfo->minRowBytes();
    if (!dstPixels || dstRowBytes < minRowBytes || dstInfo->isEmpty()) {
        return false;
    }
    auto context = std::make_shared<SyncReadContext>();

    SkIRect srcRect = SkIRect::MakeXYWH(srcX, srcY, dstInfo->width(), dstInfo->height());
    self->asyncRescaleAndReadPixels(
        surface,
        *dstInfo,
        srcRect,
        SkImage::RescaleGamma::kSrc,
        SkImage::RescaleMode::kNearest,
        &sync_read_callback,
        new std::shared_ptr<SyncReadContext>(context));

    // Force submission and block until the GPU work — and therefore the async
    // readback copy — has completed.
    if (!self->submit(skgpu::graphite::SubmitInfo(skgpu::graphite::SyncToCpu::kYes))) {
        return false;
    }

    // After a synchronous submit the finished procs run, but pump
    // checkAsyncWorkCompletion() defensively until the callback fires. Bounded so
    // an unsupported / failed readback returns false instead of hanging.
    int guard = 0;
    while (!context->fCalled.load(std::memory_order_acquire) && guard++ < 10000) {
        self->checkAsyncWorkCompletion();
    }
    // A late callback owns its state and never writes into the caller's buffer.
    if (!context->fCalled.load(std::memory_order_acquire)) {
        return false;
    }
    auto result = std::move(context->fResult);
    if (!result || result->count() != 1 || !result->data(0) || result->rowBytes(0) < minRowBytes) {
        return false;
    }
    const char* source = static_cast<const char*>(result->data(0));
    const size_t sourceRowBytes = result->rowBytes(0);
    for (int row = 0; row < dstInfo->height(); ++row) {
        std::memcpy(static_cast<char*>(dstPixels) + static_cast<size_t>(row) * dstRowBytes,
                    source + static_cast<size_t>(row) * sourceRowBytes,
                    minRowBytes);
    }
    return true;
}

//
// gpu/graphite/GraphiteTypes.h
//

// InsertRecordingInfo is not POD: fSimulatedStatus is an InsertStatus, which
// holds a std::string. It must be heap-allocated (new/delete) rather than
// zero-initialized or placement-constructed into Rust-owned storage: a
// libstdc++ std::string in SSO state points into itself, so Rust moving the
// struct by value would leave that pointer dangling and the destructor would
// free() an invalid pointer. The heap object never moves.
extern "C" skgpu::graphite::InsertRecordingInfo* C_InsertRecordingInfo_new() {
    return new skgpu::graphite::InsertRecordingInfo();
}

extern "C" void C_InsertRecordingInfo_delete(skgpu::graphite::InsertRecordingInfo* self) {
    delete self;
}

//
// gpu/graphite/ContextOptions.h
//

extern "C" void C_ContextOptions_Construct(skgpu::graphite::ContextOptions* uninitialized) {
    new(uninitialized) skgpu::graphite::ContextOptions();
}

extern "C" void C_ContextOptions_destruct(skgpu::graphite::ContextOptions* self) {
    self->~ContextOptions();
}

//
// gpu/graphite/Recorder.h
//

// RecorderOptions has a non-trivial constructor (e.g. fGpuBudgetInBytes defaults
// to 256 MiB, an sk_sp member, std::optional) and destructor, so it must be
// placement-constructed and destructed rather than zero-initialized.
extern "C" void C_RecorderOptions_Construct(skgpu::graphite::RecorderOptions* uninitialized) {
    new(uninitialized) skgpu::graphite::RecorderOptions();
}

extern "C" void C_RecorderOptions_destruct(skgpu::graphite::RecorderOptions* self) {
    self->~RecorderOptions();
}

extern "C" skgpu::graphite::Recording* C_Recorder_snap(skgpu::graphite::Recorder* self) {
    auto recording = self->snap();
    auto found = fantaProviders.find(self);
    if (found != fantaProviders.end()) {
        auto* provider = found->second;
        if (provider->conversionFailed()) {
            provider->discardRecording();
            std::fprintf(stderr, "Fanta Graphite: image texture conversion failed; discarding recording\n");
            return nullptr;
        }
        if (!recording) {
            provider->discardRecording();
        }
    }
    return recording.release();
}



extern "C" SkCanvas* C_Recorder_makeDeferredCanvas(skgpu::graphite::Recorder* self, const SkImageInfo* imageInfo, const skgpu::graphite::TextureInfo* textureInfo) {
    return self->makeDeferredCanvas(*imageInfo, *textureInfo);
}

extern "C" skgpu::BackendApi C_Recorder_backend(const skgpu::graphite::Recorder* self) {
    return self->backend();
}

// skgpu::graphite::Recorder is owned via std::unique_ptr (Context::makeRecorder
// returns unique_ptr and the shim releases it). It is NOT ref-counted, so the
// Rust wrapper must `delete` it.
extern "C" void C_Recorder_delete(skgpu::graphite::Recorder* self) {
    fantaProviders.erase(self);
    delete self;
}

//
// gpu/graphite/Recording.h
//

extern "C" void C_Recording_delete(const skgpu::graphite::Recording* self) {
    delete self;
}

//
// core/SkCanvas.h (Graphite-specific extensions)
//

extern "C" skgpu::graphite::Recorder* C_SkCanvas_recorder(const SkCanvas* self) {
    return self->recorder();
}

//
// gpu/graphite/Surface.h
//

extern "C" SkSurface* C_SkSurfaces_RenderTargetGraphite(
    skgpu::graphite::Recorder* recorder,
    const SkImageInfo* imageInfo,
    skgpu::Mipmapped mipmapped,
    const SkSurfaceProps* props,
    const char* label) {
    return SkSurfaces::RenderTarget(
            recorder,
            *imageInfo,
            mipmapped,
            props,
            label ? std::string_view(label) : std::string_view()).release();
}

extern "C" SkSurface* C_SkSurfaces_WrapBackendTextureGraphite(
        skgpu::graphite::Recorder* recorder,
        const skgpu::graphite::BackendTexture* backendTexture,
        SkColorType colorType,
        SkColorSpace* colorSpace,
        const SkSurfaceProps* surfaceProps) {
    return SkSurfaces::WrapBackendTexture(
            recorder,
            *backendTexture,
            colorType,
            sp(colorSpace),
            surfaceProps).release();
}

extern "C" SkImage* C_SkSurfaces_AsImageGraphite(SkSurface* surface) {
    return SkSurfaces::AsImage(sp(surface)).release();
}

extern "C" SkImage* C_SkSurfaces_AsImageCopyGraphite(
    SkSurface* surface,
    const SkIRect* subset,
    skgpu::Mipmapped mipmapped) {
    return SkSurfaces::AsImageCopy(
            sp(surface),
            subset,
            mipmapped).release();
}

//
// gpu/graphite/Image.h
//

extern "C" SkImage* C_SkImages_WrapTextureGraphite(
        skgpu::graphite::Recorder* recorder,
        const skgpu::graphite::BackendTexture* backendTexture,
        SkColorType colorType,
        SkAlphaType alphaType,
        SkColorSpace* colorSpace) {
    return SkImages::WrapTexture(
            recorder,
            *backendTexture,
            colorType,
            alphaType,
            sp(colorSpace)).release();
}

extern "C" SkImage* C_SkImages_TextureFromImageGraphite(
        skgpu::graphite::Recorder* recorder,
        const SkImage* image) {
    return SkImages::TextureFromImage(
            recorder,
            image).release();
}

extern "C" SkImage* C_SkImages_SubsetTextureFromGraphite(
    skgpu::graphite::Recorder* recorder,
    const SkImage* image,
    const SkIRect* subset) {
    return SkImages::SubsetTextureFrom(
            recorder,
            image,
            *subset).release();
}

//
// gpu/graphite/mtl/MtlBackendContext.h
//

#ifdef SK_METAL
extern "C" void C_MtlBackendContext_Construct(
    skgpu::graphite::MtlBackendContext* uninitialized,
    const void* device, const void* queue) {
    new (uninitialized) skgpu::graphite::MtlBackendContext();
    uninitialized->fDevice.retain(static_cast<CFTypeRef>(const_cast<void*>(device)));
    uninitialized->fQueue.retain(static_cast<CFTypeRef>(const_cast<void*>(queue)));
}

extern "C" void C_MtlBackendContext_destruct(skgpu::graphite::MtlBackendContext* self) {
    self->~MtlBackendContext();
}

extern "C" skgpu::graphite::Context* C_ContextFactory_MakeMetal(
    const skgpu::graphite::MtlBackendContext* backendContext,
    const skgpu::graphite::ContextOptions* options) {
    return skgpu::graphite::ContextFactory::MakeMetal(*backendContext, *options).release();
}

extern "C" SkContext* C_SkContexts_MakeGraphiteMetal(
    const skgpu::graphite::MtlBackendContext* backendContext,
    const SkContextOptions* options) {
    return SkContexts::MakeGraphite(*backendContext, *options).release();
}

extern "C" void C_BackendTextures_MakeMetal(
    skgpu::graphite::BackendTexture* uninitialized,
    int width,
    int height,
    const void* mtlTexture) {
    new(uninitialized) skgpu::graphite::BackendTexture(
        skgpu::graphite::BackendTextures::MakeMetal(
            SkISize::Make(width, height),
            static_cast<CFTypeRef>(const_cast<void*>(mtlTexture))
        )
    );
}
#endif

#ifdef SK_VULKAN
// skgpu::VulkanBackendContext is the backend-context type shared with Ganesh
// (already wrapped by skia_safe::gpu::vk::BackendContext). The graphite Vulkan
// context factory lives in the skgpu::graphite::ContextFactory namespace,
// mirroring MakeMetal above. The backend context is passed as an opaque pointer
// to match the existing gpu::vk binding, which keeps it as a heap void*.
extern "C" skgpu::graphite::Context* C_ContextFactory_MakeVulkan(
    const void* backendContext,
    const skgpu::graphite::ContextOptions* options) {
    return skgpu::graphite::ContextFactory::MakeVulkan(
        *static_cast<const skgpu::VulkanBackendContext*>(backendContext), *options).release();
}

extern "C" SkContext* C_SkContexts_MakeGraphiteVulkan(
    const void* backendContext,
    const SkContextOptions* options) {
    return SkContexts::MakeGraphite(
        *static_cast<const skgpu::VulkanBackendContext*>(backendContext), *options).release();
}
#endif
