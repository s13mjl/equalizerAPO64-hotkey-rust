#define NOMINMAX
#include <windows.h>
#include <objbase.h>
#include <audioenginebaseapo.h>
#include <baseaudioprocessingobject.h>
#include <strsafe.h>
#include <new>
#include <string>

#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "uuid.lib")

static const CLSID CLSID_EqualizerApoPreMix =
    {0xeacd2258, 0xfcac, 0x4ff4, {0xb3, 0x6d, 0x41, 0x9e, 0x92, 0x4a, 0x6d, 0x79}};
static const CLSID CLSID_EqualizerApoPostMix =
    {0xec1cc9ce, 0xfaed, 0x4822, {0x82, 0x8a, 0x82, 0xa8, 0x1a, 0x6f, 0x01, 0x8f}};
// Verification-only class id, compiled in only with EAPO_LIVE_PROBE (cargo
// feature `live-probe`): the audio engine caches a class object per CLSID, so a
// fresh CLSID is the only way to load our DLL into an already running
// audiodg.exe without restarting the audio service. Keep in sync with
// EQUALIZERAPO_PROBE_GUID in src/apo.rs.
#ifdef EAPO_LIVE_PROBE
static const CLSID CLSID_EqualizerApoProbe =
    {0xd3a9e4b1, 0x6c25, 0x4f7a, {0x9e, 0x83, 0x1b, 0x5c, 0x7d, 0x2a, 0x4e, 0x60}};
#endif

extern "C" HRESULT __stdcall eapo_rust_create_inner(REFCLSID clsid, void* cxxBase, void** ppv);

// Diagnostic build only (cargo feature `live-probe`): records the audio engine's
// call sequence on the object it owns, so a live capture can show which methods
// are called, with what result, and when the object dies. Deliberately absent
// from QueryInterface/AddRef, where logging from inside a graph host destabilised
// earlier captures.
#ifdef EAPO_LIVE_PROBE
#include <cstdio>
extern "C" void eapo_rust_probe_log(const char* text);
#define PROBE(...)                                                       \
    do {                                                                 \
        char eapoProbeBuf[256];                                          \
        snprintf(eapoProbeBuf, sizeof(eapoProbeBuf), __VA_ARGS__);       \
        eapo_rust_probe_log(eapoProbeBuf);                               \
    } while (false)
#else
#define PROBE(...) ((void)0)
#endif

class INonDelegatingUnknown
{
public:
    virtual HRESULT __stdcall NonDelegatingQueryInterface(const IID& iid, void** ppv) = 0;
    virtual ULONG __stdcall NonDelegatingAddRef() = 0;
    virtual ULONG __stdcall NonDelegatingRelease() = 0;
};

// Mirrors `EqualizerAPO::regPostMixProperties`: the WDK template fills the blob
// in a static initializer, which runs under the loader lock long before any COM
// call can reach it. Filling it lazily would let one thread `memset` the blob
// while the engine copies it on another thread.
static const CRegAPOProperties<1> g_postMixProperties(
    CLSID_EqualizerApoPostMix, L"EqualizerAPO", L"Copyright (C) 2015", 1, 0,
    __uuidof(IAudioProcessingObject),
    (APO_FLAG)(APO_FLAG_FRAMESPERSECOND_MUST_MATCH | APO_FLAG_BITSPERSAMPLE_MUST_MATCH |
               APO_FLAG_INPLACE));
static LONG g_instanceCount = 0;
static LONG g_lockCount = 0;

/// The blob the WDK base class is constructed with. `EqualizerAPO`'s constructor
/// is `CBaseAudioProcessingObject(regPostMixProperties)` for *every* class id it
/// is created for, so a pre-mix instance reports the post-mix CLSID from
/// `GetRegistrationProperties`. That is what the audio engine reads, and it must
/// not be "corrected" here. The per-class-id blobs used by `DllRegisterServer`
/// are built on the Rust side.
static const APO_REG_PROPERTIES* baseProperties()
{
    return static_cast<const APO_REG_PROPERTIES*>(g_postMixProperties);
}

class RustApo final : public CBaseAudioProcessingObject,
                      public IAudioSystemEffects,
                      public INonDelegatingUnknown
{
public:
    RustApo(IUnknown* outer, REFCLSID clsid)
        : CBaseAudioProcessingObject(baseProperties()),
          refCount(1),
          pUnkOuter(outer ? outer
                          : reinterpret_cast<IUnknown*>(
                                static_cast<INonDelegatingUnknown*>(this))),
          innerApo(nullptr),
          innerRT(nullptr),
          innerCfg(nullptr)
    {
        if (FAILED(eapo_rust_create_inner(clsid, this, reinterpret_cast<void**>(&innerApo)))) {
            innerApo = nullptr;
        }
        if (innerApo != nullptr) {
            innerApo->QueryInterface(__uuidof(IAudioProcessingObjectRT),
                                     reinterpret_cast<void**>(&innerRT));
            innerApo->QueryInterface(__uuidof(IAudioProcessingObjectConfiguration),
                                     reinterpret_cast<void**>(&innerCfg));
        }
        InterlockedIncrement(&g_instanceCount);
        PROBE("ctor this=%p clsid1=%08X inner=%p rt=%p cfg=%p", this,
              static_cast<unsigned>(clsid.Data1), innerApo, innerRT, innerCfg);
    }

    ~RustApo()
    {
        PROBE("dtor this=%p", this);
        if (innerCfg != nullptr) {
            innerCfg->Release();
            innerCfg = nullptr;
        }
        if (innerRT != nullptr) {
            innerRT->Release();
            innerRT = nullptr;
        }
        if (innerApo != nullptr) {
            innerApo->Release();
            innerApo = nullptr;
        }
        InterlockedDecrement(&g_instanceCount);
    }

    HRESULT __stdcall QueryInterface(const IID& iid, void** ppv) override
    {
        return pUnkOuter->QueryInterface(iid, ppv);
    }

    // `EqualizerAPO::AddRef`/`Release` are `pUnkOuter->AddRef()`/`Release()`, not
    // a private counter. When the engine aggregates the APO (its
    // `CSystemEffectWrapper` passes itself as `pUnkOuter`), a private counter
    // leaves the outer object's count untouched, so the engine frees it while it
    // is still referenced and later faults adding a reference to freed memory.
    // With the non-aggregated case `pUnkOuter` is the `INonDelegatingUnknown`
    // sub-object, whose slots land on `NonDelegatingAddRef`/`Release` — the same
    // counter, exactly like the C++ original.
    ULONG __stdcall AddRef() override
    {
        return pUnkOuter->AddRef();
    }

    ULONG __stdcall Release() override
    {
        return pUnkOuter->Release();
    }

    HRESULT __stdcall NonDelegatingQueryInterface(const IID& iid, void** ppv) override
    {
        if (ppv == nullptr) {
            return E_POINTER;
        }
        if (iid == __uuidof(IUnknown)) {
            *ppv = static_cast<INonDelegatingUnknown*>(this);
        } else if (iid == __uuidof(IAudioProcessingObject)) {
            *ppv = static_cast<IAudioProcessingObject*>(this);
        } else if (iid == __uuidof(IAudioProcessingObjectRT)) {
            *ppv = static_cast<IAudioProcessingObjectRT*>(this);
        } else if (iid == __uuidof(IAudioProcessingObjectConfiguration)) {
            *ppv = static_cast<IAudioProcessingObjectConfiguration*>(this);
        } else if (iid == __uuidof(IAudioSystemEffects)) {
            *ppv = static_cast<IAudioSystemEffects*>(this);
        } else {
            *ppv = nullptr;
            return E_NOINTERFACE;
        }
        reinterpret_cast<IUnknown*>(*ppv)->AddRef();
        return S_OK;
    }

    ULONG __stdcall NonDelegatingAddRef() override
    {
        return static_cast<ULONG>(InterlockedIncrement(&refCount));
    }

    ULONG __stdcall NonDelegatingRelease() override
    {
        LONG result = InterlockedDecrement(&refCount);
        if (result == 0) {
            PROBE("release0 this=%p", this);
            delete this;
            return 0;
        }
        return static_cast<ULONG>(result);
    }

    HRESULT __stdcall GetLatency(HNSTIME* pTime) override
    {
        // Mirrors EqualizerAPO::GetLatency, which the WDK base class cannot
        // provide: an unlocked object must report APOERR_ALREADY_UNLOCKED, and a
        // locked one adds its child APO's latency.
        if (pTime == nullptr) {
            return E_POINTER;
        }
        if (!m_bIsLocked) {
            PROBE("GetLatency this=%p hr=887D0006 (unlocked)", this);
            return APOERR_ALREADY_UNLOCKED;
        }
        *pTime = 0;
        if (innerApo != nullptr) {
            innerApo->GetLatency(pTime);
        }
        PROBE("GetLatency this=%p value=%lld", this, static_cast<long long>(*pTime));
        return S_OK;
    }

#ifdef EAPO_LIVE_PROBE
    // Diagnostics only: each override forwards straight to the WDK base and
    // records that the audio engine called it, so behavior is unchanged.
    HRESULT __stdcall GetRegistrationProperties(APO_REG_PROPERTIES** ppRegProps) override
    {
        HRESULT hr = CBaseAudioProcessingObject::GetRegistrationProperties(ppRegProps);
        PROBE("GetRegistrationProperties this=%p hr=%08X clsid1=%08X", this,
              static_cast<unsigned>(hr),
              (ppRegProps != nullptr && *ppRegProps != nullptr)
                  ? static_cast<unsigned>((*ppRegProps)->clsid.Data1)
                  : 0u);
        return hr;
    }

    HRESULT __stdcall Reset() override
    {
        HRESULT hr = CBaseAudioProcessingObject::Reset();
        PROBE("Reset this=%p hr=%08X", this, static_cast<unsigned>(hr));
        return hr;
    }

    HRESULT __stdcall GetInputChannelCount(UINT32* pu32ChannelCount) override
    {
        HRESULT hr = CBaseAudioProcessingObject::GetInputChannelCount(pu32ChannelCount);
        PROBE("GetInputChannelCount this=%p hr=%08X count=%u", this,
              static_cast<unsigned>(hr), pu32ChannelCount != nullptr ? *pu32ChannelCount : 0u);
        return hr;
    }

    HRESULT __stdcall IsOutputFormatSupported(IAudioMediaType* pInputFormat,
                                              IAudioMediaType* pRequestedOutputFormat,
                                              IAudioMediaType** ppSupportedOutputFormat) override
    {
        HRESULT hr = CBaseAudioProcessingObject::IsOutputFormatSupported(
            pInputFormat, pRequestedOutputFormat, ppSupportedOutputFormat);
        PROBE("IsOutputFormatSupported this=%p hr=%08X supported=%p", this,
              static_cast<unsigned>(hr),
              ppSupportedOutputFormat != nullptr ? *ppSupportedOutputFormat : nullptr);
        return hr;
    }
#endif

    HRESULT __stdcall Initialize(UINT32 cbDataSize, BYTE* pbyData) override
    {
        if (innerApo == nullptr) {
            PROBE("Initialize this=%p cb=%u no-inner hr=80004005", this, cbDataSize);
            return E_FAIL;
        }
        HRESULT hr = innerApo->Initialize(cbDataSize, pbyData);
        PROBE("Initialize this=%p cb=%u hr=%08X", this, cbDataSize, static_cast<unsigned>(hr));
        return hr;
    }

    HRESULT __stdcall IsInputFormatSupported(IAudioMediaType* outputFormat,
                                             IAudioMediaType* requestedInputFormat,
                                             IAudioMediaType** supportedInputFormat) override
    {
        HRESULT hr;
        if (innerApo != nullptr) {
            hr = innerApo->IsInputFormatSupported(outputFormat, requestedInputFormat,
                                                  supportedInputFormat);
        }
        else {
            hr = CBaseAudioProcessingObject::IsInputFormatSupported(
                outputFormat, requestedInputFormat, supportedInputFormat);
        }
        PROBE("IsInputFormatSupported this=%p hr=%08X supported=%p", this,
              static_cast<unsigned>(hr),
              supportedInputFormat ? *supportedInputFormat : nullptr);
        return hr;
    }

    HRESULT __stdcall LockForProcess(UINT32 numInputConnections,
                                     APO_CONNECTION_DESCRIPTOR** inputConnections,
                                     UINT32 numOutputConnections,
                                     APO_CONNECTION_DESCRIPTOR** outputConnections) override
    {
        if (innerCfg == nullptr) {
            PROBE("LockForProcess this=%p no-cfg hr=80004005", this);
            return E_FAIL;
        }
        // The Rust object calls CBaseAudioProcessingObject::LockForProcess
        // through eapo_cxx_base_lock_for_process, at the same point where the
        // C++ EqualizerAPO::LockForProcess calls it.
        HRESULT hr = innerCfg->LockForProcess(numInputConnections, inputConnections,
                                              numOutputConnections, outputConnections);
        PROBE("LockForProcess this=%p in=%u out=%u hr=%08X", this, numInputConnections,
              numOutputConnections, static_cast<unsigned>(hr));
        return hr;
    }

    HRESULT __stdcall UnlockForProcess() override
    {
        if (innerCfg == nullptr) {
            return E_FAIL;
        }
        HRESULT hr = innerCfg->UnlockForProcess();
        PROBE("UnlockForProcess this=%p hr=%08X", this, static_cast<unsigned>(hr));
        return hr;
    }

    void __stdcall APOProcess(UINT32 numInputConnections,
                              APO_CONNECTION_PROPERTY** inputConnections,
                              UINT32 numOutputConnections,
                              APO_CONNECTION_PROPERTY** outputConnections) override
    {
        if (innerRT != nullptr) {
            innerRT->APOProcess(numInputConnections, inputConnections,
                                numOutputConnections, outputConnections);
        }
    }

private:
    LONG refCount;
    IUnknown* pUnkOuter;
    IAudioProcessingObject* innerApo;
    IAudioProcessingObjectRT* innerRT;
    IAudioProcessingObjectConfiguration* innerCfg;
};

class ClassFactory final : public IClassFactory
{
public:
    explicit ClassFactory(REFCLSID clsid) : refCount(1), clsid(clsid) {}

    HRESULT __stdcall QueryInterface(const IID& iid, void** ppv) override
    {
        if (ppv == nullptr) {
            return E_POINTER;
        }
        if (iid == __uuidof(IUnknown) || iid == __uuidof(IClassFactory)) {
            *ppv = static_cast<IClassFactory*>(this);
        } else {
            *ppv = nullptr;
            return E_NOINTERFACE;
        }
        reinterpret_cast<IUnknown*>(*ppv)->AddRef();
        return S_OK;
    }

    ULONG __stdcall AddRef() override
    {
        return static_cast<ULONG>(InterlockedIncrement(&refCount));
    }

    ULONG __stdcall Release() override
    {
        LONG result = InterlockedDecrement(&refCount);
        if (result == 0) {
            delete this;
            return 0;
        }
        return static_cast<ULONG>(result);
    }

    HRESULT __stdcall CreateInstance(IUnknown* outer, const IID& iid, void** ppv) override
    {
        if (outer != nullptr && iid != __uuidof(IUnknown)) {
            return E_NOINTERFACE;
        }
        RustApo* apo = new (std::nothrow) RustApo(outer, clsid);
        if (apo == nullptr) {
            return E_OUTOFMEMORY;
        }
        HRESULT hr = apo->NonDelegatingQueryInterface(iid, ppv);
        apo->NonDelegatingRelease();
        return hr;
    }

    HRESULT __stdcall LockServer(BOOL lock) override
    {
        if (lock) {
            InterlockedIncrement(&g_lockCount);
        } else {
            InterlockedDecrement(&g_lockCount);
        }
        return S_OK;
    }

private:
    LONG refCount;
    CLSID clsid;
};

extern "C" HRESULT __stdcall
eapo_cxx_get_class_object(const CLSID* clsid, const IID* iid, void** ppv)
{
    if (clsid == nullptr || iid == nullptr) {
        return E_POINTER;
    }
    if (ppv == nullptr) {
        return E_POINTER;
    }
    *ppv = nullptr;
    if (!IsEqualCLSID(*clsid, CLSID_EqualizerApoPreMix) &&
        !IsEqualCLSID(*clsid, CLSID_EqualizerApoPostMix)
#ifdef EAPO_LIVE_PROBE
        && !IsEqualCLSID(*clsid, CLSID_EqualizerApoProbe)
#endif
        ) {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    ClassFactory* factory = new (std::nothrow) ClassFactory(*clsid);
    if (factory == nullptr) {
        return E_OUTOFMEMORY;
    }
    HRESULT hr = factory->QueryInterface(*iid, ppv);
    factory->Release();
    return hr;
}

extern "C" HRESULT __stdcall eapo_cxx_can_unload_now()
{
    return (g_instanceCount == 0 && g_lockCount == 0) ? S_OK : S_FALSE;
}

// Entry points the Rust object uses to reach the WDK base class it inherits.
extern "C" HRESULT __stdcall eapo_cxx_base_is_format_supported(
    void* wrapper,
    IAudioMediaType* pOppositeFormat,
    IAudioMediaType* pRequestedFormat,
    IAudioMediaType** ppSupportedFormat,
    bool bIsInput)
{
    if (wrapper == nullptr) {
        return E_POINTER;
    }
    RustApo* apo = static_cast<RustApo*>(wrapper);
    if (bIsInput) {
        return apo->CBaseAudioProcessingObject::IsInputFormatSupported(
            pOppositeFormat, pRequestedFormat, ppSupportedFormat);
    }
    return apo->CBaseAudioProcessingObject::IsOutputFormatSupported(
        pOppositeFormat, pRequestedFormat, ppSupportedFormat);
}

extern "C" HRESULT __stdcall eapo_cxx_base_lock_for_process(
    void* wrapper,
    UINT32 numInputConnections,
    APO_CONNECTION_DESCRIPTOR** ppInputConnections,
    UINT32 numOutputConnections,
    APO_CONNECTION_DESCRIPTOR** ppOutputConnections)
{
    if (wrapper == nullptr) {
        return E_POINTER;
    }
    RustApo* apo = static_cast<RustApo*>(wrapper);
    return apo->CBaseAudioProcessingObject::LockForProcess(
        numInputConnections, ppInputConnections, numOutputConnections, ppOutputConnections);
}

extern "C" HRESULT __stdcall eapo_cxx_base_unlock_for_process(void* wrapper)
{
    if (wrapper == nullptr) {
        return E_POINTER;
    }
    RustApo* apo = static_cast<RustApo*>(wrapper);
    return apo->CBaseAudioProcessingObject::UnlockForProcess();
}
