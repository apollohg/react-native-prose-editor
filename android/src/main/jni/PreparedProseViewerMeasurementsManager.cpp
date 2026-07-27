#include "reactnativeproseeditor.h"

#include <fbjni/fbjni.h>
#include <folly/dynamic.h>
#include <limits>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <react/jni/ReadableNativeMap.h>
#include <react/renderer/core/conversions.h>

using namespace facebook::jni;

namespace facebook::react {

namespace {

folly::dynamic optionalStringToDynamic(
    const std::optional<std::string>& value) {
  return value ? folly::dynamic(*value) : folly::dynamic(nullptr);
}

folly::dynamic toDynamic(const PreparedProseViewerProps& props) {
  // The generated component props are deliberately copied into a ReadableMap
  // for FabricUIManager.measure; the Java manager is the only Android layout
  // boundary and owns DIP-to-pixel normalization.
  return folly::dynamic::object
      ("sourceKind", props.sourceKind == PreparedProseViewerSourceKind::Html ? "html" : "json")
      ("source", props.source)
      ("configJson", props.configJson)
      ("themeJson", optionalStringToDynamic(props.themeJson))
      ("imagePolicyJson", optionalStringToDynamic(props.imagePolicyJson))
      ("imagesEnabled", props.imagesEnabled)
      ("collapsesWhenEmpty", props.collapsesWhenEmpty)
      ("enableLinkTaps", props.enableLinkTaps)
      ("fontEnvironmentRevision", props.fontEnvironmentRevision);
}

folly::dynamic toState(
    uint64_t attachmentRevision,
    uint64_t nativeFontRevision,
    uint64_t leaseHandle) {
  return folly::dynamic::object
      ("attachmentRevision", static_cast<int64_t>(attachmentRevision))
      ("nativeFontRevision", static_cast<int64_t>(nativeFontRevision))
      ("leaseHandle", std::to_string(static_cast<int64_t>(leaseHandle)));
}

// Each C++ state-family owns one bridge while it is live. Terminal callbacks
// can run on the final C++ state release rather than a Java-created thread, so
// this must retain a JNI global reference instead of the local class lookup
// that created it. The bridge owns no state-family object, avoiding a cycle.
class AndroidLeaseLifecycleBridge final {
 public:
  static AndroidLeaseLifecycleBridge& processLifetime() {
    // Intentionally process-lifetime: VM teardown can occur from an
    // unattached native thread, where destroying a JNI global is unsafe.
    static std::mutex mutex;
    static AndroidLeaseLifecycleBridge* bridge = nullptr;
    std::lock_guard<std::mutex> lock(mutex);
    if (!bridge) {
      bridge = new AndroidLeaseLifecycleBridge(
          facebook::jni::make_global(facebook::jni::findClassStatic(
              "com/apollohg/editor/viewer/FabricLeaseHandleBridge")));
    }
    return *bridge;
  }

  explicit AndroidLeaseLifecycleBridge(
      facebook::jni::global_ref<facebook::jni::JClass> bridgeClass)
      : bridgeClass_(std::move(bridgeClass)),
        registerLease_(bridgeClass_->getStaticMethod<void(jint, jint, jlong)>(
            "registerNativeLease")),
        finalizeLease_(bridgeClass_->getStaticMethod<void(jint, jint, jlong)>(
            "finalizeNativeLease")) {}

  void registerLease(SurfaceId surfaceId, Tag componentTag, uint64_t leaseHandle) const {
    registerLease_(bridgeClass_, static_cast<jint>(surfaceId),
                   static_cast<jint>(componentTag),
                   static_cast<jlong>(leaseHandle));
  }

  void finalizeLease(SurfaceId surfaceId, Tag componentTag, uint64_t leaseHandle) const {
    finalizeLease_(bridgeClass_, static_cast<jint>(surfaceId),
                   static_cast<jint>(componentTag),
                   static_cast<jlong>(leaseHandle));
  }

 private:
  facebook::jni::global_ref<facebook::jni::JClass> bridgeClass_;
  facebook::jni::JStaticMethod<void(jint, jint, jlong)> registerLease_;
  facebook::jni::JStaticMethod<void(jint, jint, jlong)> finalizeLease_;
};

} // namespace

void PreparedProseMeasurementsManager::bindLeaseLifecycle(
    SurfaceId surfaceId,
    Tag componentTag,
    uint64_t leaseHandle,
    const std::shared_ptr<PreparedProseViewerLeaseLifecycle>& leaseLifecycle) const {
  if (leaseHandle == 0 || !leaseLifecycle || !leaseLifecycle->isActive()) {
    return;
  }
  // Yoga may run on a native worker rather than directly beneath a JNI entry
  // point. ThreadScope gives the one-time class lookup an attached env and
  // caches it for this native call without retaining the thread afterward.
  if (!facebook::jni::Environment::isGlobalJvmAvailable()) {
    return;
  }
  try {
    facebook::jni::ThreadScope threadScope;
    auto& bridge = AndroidLeaseLifecycleBridge::processLifetime();
    bridge.registerLease(surfaceId, componentTag, leaseHandle);
    leaseLifecycle->bindTerminalCleanup([surfaceId, componentTag, leaseHandle] {
      // The class global is intentionally process-lifetime. Runtime-unavailable
      // paths no-op; they never reset or destroy it without an attached env.
      try {
        if (!facebook::jni::Environment::isGlobalJvmAvailable()) {
          return;
        }
        facebook::jni::ThreadScope threadScope;
        try {
          AndroidLeaseLifecycleBridge::processLifetime().finalizeLease(
              surfaceId, componentTag, leaseHandle);
        } catch (...) {
          // The Java registry may already be gone during runtime teardown.
        }
      } catch (...) {
        // A failed attach means VM teardown is already underway. The process
        // lifetime bridge is deliberately left untouched.
      }
    });
  } catch (...) {
    // Runtime teardown may race a Yoga worker before it has an attached env.
    // Do not bind a terminal Java callback from a partially destroyed VM.
  }
}

Size PreparedProseMeasurementsManager::measure(
    SurfaceId surfaceId,
    Tag componentTag,
    const PreparedProseViewerProps& props,
    Float effectiveWidth,
    Float /*pointScaleFactor*/,
    uint64_t attachmentRevision,
    uint64_t nativeFontRevision,
    double nativeFontScale,
    uint64_t /*fontEnvironmentRevision*/,
    uint64_t leaseHandle,
    const std::shared_ptr<PreparedProseViewerLeaseLifecycle>& /*leaseLifecycle*/) const {
  // Every object allocation, class lookup, and Java invocation below can run
  // from a Yoga worker.  It must live in this one attached scope; binding the
  // lifecycle above is insufficient because its scope has already ended.
  if (!facebook::jni::Environment::isGlobalJvmAvailable()) {
    return {};
  }
  try {
    facebook::jni::ThreadScope threadScope;
    const auto& fabricUIManager =
        contextContainer_->at<jni::global_ref<jobject>>("FabricUIManager");
    static auto measure = facebook::jni::findClassStatic(
                              "com/facebook/react/fabric/FabricUIManager")
                              ->getMethod<jlong(
                                  jint,
                                  jstring,
                                  ReadableMap::javaobject,
                                  ReadableMap::javaobject,
                                  ReadableMap::javaobject,
                                  jfloat,
                                  jfloat,
                                  jfloat,
                                  jfloat)>("measure");
    static auto beginNativeMeasure = facebook::jni::findClassStatic(
        "com/apollohg/editor/viewer/FabricLeaseHandleBridge")
        ->getStaticMethod<void(jlong)>("beginNativeMeasure");
    static auto endNativeMeasure = facebook::jni::findClassStatic(
        "com/apollohg/editor/viewer/FabricLeaseHandleBridge")
        ->getStaticMethod<void()>("endNativeMeasure");
    const auto localData = folly::dynamic::object
        ("surfaceId", static_cast<int64_t>(surfaceId))
        ("componentTag", static_cast<int64_t>(componentTag))
        ("leaseHandle", std::to_string(static_cast<int64_t>(leaseHandle)));
    const auto propsDynamic = toDynamic(props);
    const auto stateDynamic = toState(attachmentRevision, nativeFontRevision, leaseHandle)
        ("nativeFontScale", nativeFontScale);
    const auto localDataNative = ReadableNativeMap::newObjectCxxArgs(localData);
    const auto propsNative = ReadableNativeMap::newObjectCxxArgs(propsDynamic);
    const auto stateNative = ReadableNativeMap::newObjectCxxArgs(stateDynamic);
    const auto localDataMap = make_local(reinterpret_cast<ReadableMap::javaobject>(localDataNative.get()));
    const auto propsMap = make_local(reinterpret_cast<ReadableMap::javaobject>(propsNative.get()));
    const auto stateMap = make_local(reinterpret_cast<ReadableMap::javaobject>(stateNative.get()));
    const auto componentName = make_jstring("PreparedProseViewer");
    const auto width = effectiveWidth;
    beginNativeMeasure(static_cast<int64_t>(leaseHandle));
    try {
      const auto result = yogaMeassureToSize(measure(
          fabricUIManager,
          surfaceId,
          componentName.get(),
          localDataMap.get(),
          propsMap.get(),
          stateMap.get(),
          0,
          width,
          0,
          std::numeric_limits<Float>::infinity()));
      endNativeMeasure();
      return result;
    } catch (...) {
      // Still inside ThreadScope, so this is the only safe place to clear the
      // Java thread-local handoff after FabricUIManager.measure throws.
      endNativeMeasure();
      return {};
    }
  } catch (...) {
    // JVM shutdown and failed attach are terminal for this synchronous Yoga
    // pass.  Returning an empty size is safe; never use a potentially
    // destroyed JNIEnv to try to balance the Java thread-local callback.
    return {};
  }
}

} // namespace facebook::react
