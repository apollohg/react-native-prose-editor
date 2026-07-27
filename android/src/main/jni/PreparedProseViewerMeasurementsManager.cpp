#include "reactnativeproseeditor.h"

#include <fbjni/fbjni.h>
#include <folly/dynamic.h>
#include <limits>
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

} // namespace

void PreparedProseMeasurementsManager::bindLeaseLifecycle(
    SurfaceId /*surfaceId*/,
    Tag /*componentTag*/,
    uint64_t /*leaseHandle*/,
    const std::shared_ptr<PreparedProseViewerLeaseLifecycle>& /*leaseLifecycle*/) const {
  // Android's ViewManager receives an explicit onSurfaceStopped callback.
  // The handle still scopes every per-view registry operation below.
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
    endNativeMeasure();
    throw;
  }
}

} // namespace facebook::react
